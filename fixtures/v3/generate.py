#!/usr/bin/env python3
"""Generate public, deterministic M0 vectors. Never use these keys in a room.

Requires cbor2 and cryptography. Rust consumes the checked-in output without
running this script. cbor2 and Rust/ciborium are independent implementations.
"""
import base64
import copy
import hashlib
import json
from pathlib import Path

import cbor2
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

ROOT = Path(__file__).parent
SEED = bytes(range(32))
KEY = Ed25519PrivateKey.from_private_bytes(SEED)
PUB = KEY.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
NOW = 1_800_000_000_000


def cbor(value):
    # All maps in this profile have text keys; cbor2's length-first canonical
    # order and RFC8949 core deterministic bytewise order coincide for them.
    return cbor2.dumps(value, canonical=True)


def b64(value):
    return base64.b64encode(value).decode().rstrip("=")


def write(name, value):
    (ROOT / name).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def request(name, target="/rooms/test/messages?a=%2F&a=/", stamp=NOW, outcome="accept"):
    inputs = ["GET", target, hashlib.sha256(b"").digest(), stamp, bytes(range(16))]
    raw = cbor(inputs)
    signing = b"cowchat/v3/request" + raw
    return dict(id=name, cbor_hex=raw.hex(), signing_hex=signing.hex(),
                public_key_hex=PUB.hex(), signature_hex=KEY.sign(signing).hex(),
                now_ms=NOW, already_seen=False, expected=outcome)


requests = [request("raw-query-duplicates"), request("future-boundary", stamp=NOW + 300_000),
            request("past-boundary", stamp=NOW - 300_000),
            request("stale", stamp=NOW - 300_001, outcome="timestamp"),
            request("future", stamp=NOW + 300_001, outcome="timestamp")]
for name, edit, expected in [
    ("query-reencoded", lambda f: f.__setitem__(1, "/rooms/test/messages?a=/&a=/"), "signature"),
    ("query-reordered", lambda f: f.__setitem__(1, "/rooms/test/messages?a=/&a=%2F"), "signature"),
    ("target-changed", lambda f: f.__setitem__(1, "/rooms/other/messages?a=%2F&a=/"), "signature"),
]:
    item = request(name)
    fields = cbor2.loads(bytes.fromhex(item["cbor_hex"]))
    edit(fields)
    item["cbor_hex"] = cbor(fields).hex()
    item["expected"] = expected
    requests.append(item)
item = request("duplicate-nonce", outcome="replay")
item["already_seen"] = True
requests.append(item)
item = request("wrong-domain", outcome="signature")
item["signature_hex"] = KEY.sign(b"cowchat/v3/envelope" + bytes.fromhex(item["cbor_hex"])).hex()
requests.append(item)
item = request("nonminimal-array", outcome="encoding")
item["cbor_hex"] = "9805" + item["cbor_hex"][2:]
requests.append(item)
for name, index, value in [("short-nonce", 4, b"bad"), ("text-body-hash", 2, "00" * 32),
                           ("lowercase-method", 0, "get"), ("absolute-target", 1, "https://example.test/")]:
    item = request(name, outcome="schema")
    fields = cbor2.loads(bytes.fromhex(item["cbor_hex"]))
    fields[index] = value
    item["cbor_hex"] = cbor(fields).hex()
    item["signing_hex"] = (b"cowchat/v3/request" + cbor(fields)).hex()
    item["signature_hex"] = KEY.sign(bytes.fromhex(item["signing_hex"])).hex()
    requests.append(item)
write("requests.json", requests)

header = dict(v=3, message_id="00000000-0000-4000-8000-000000000001", chain_id=1,
              room="test-room", seat="0x" + "11" * 20, role="owner", via="dashboard",
              via_sender=None, **{"class": "message"}, reply_to=None,
              mentions=["0x" + "11" * 20 + "#builder"], wake_hint="normal", gen=2,
              cert="test-cert", nonce=b64(bytes(range(12))))
secret = bytes(range(32, 64))
plain = b'{"parts":[{"text":"public fixture"}],"metadata":{}}'


def envelope(name, h):
    aad = cbor(h)
    key = HKDF(algorithm=hashes.SHA256(), length=32, salt=None,
               info=b"cowchat-e2e-v1:" + h["room"].encode()).derive(secret)
    nonce = base64.b64decode(h["nonce"] + "=" * (-len(h["nonce"]) % 4))
    body = "cow1:" + b64(nonce + ChaCha20Poly1305(key).encrypt(nonce, plain, aad))
    signing = b"cowchat/v3/envelope" + aad + hashlib.sha256(body.encode()).digest()
    return dict(id=name, header=h, header_cbor_hex=aad.hex(), body=body,
                public_key_hex=PUB.hex(), signature_hex=KEY.sign(signing).hex(),
                signing_hex=signing.hex(), public_test_secret_hex=secret.hex(),
                plaintext_hex=plain.hex(), expected="accept")


envelopes = [envelope("owner-message", header)]
thinking = copy.deepcopy(header)
thinking.update({"class": "thinking", "wake_hint": "none"})
envelopes.append(envelope("thinking-no-wake", thinking))
forwarded = copy.deepcopy(header)
forwarded.update(via="telegram", via_sender="12345")
envelopes.append(envelope("owner-via-door-header", forwarded))
for name, field, value in [("cross-room", "room", "other-room"), ("cross-chain", "chain_id", 2),
                           ("altered-role", "role", "builder"), ("altered-via", "via", "telegram"),
                           ("altered-mentions", "mentions", []), ("altered-gen", "gen", 3)]:
    item = copy.deepcopy(envelopes[0])
    item["id"], item["expected"] = name, "signature"
    item["header"][field] = value
    item["header_cbor_hex"] = cbor(item["header"]).hex()
    envelopes.append(item)
for name, edit in [("omitted-null", lambda h: h.pop("via_sender")),
                    ("unknown-field", lambda h: h.update(extra=None)),
                    ("thinking-wakes", lambda h: h.update({"class": "thinking"})),
                    ("unknown-class", lambda h: h.update({"class": "notice"})),
                    ("unknown-role", lambda h: h.update(role="sheriff")),
                    ("unknown-wake-hint", lambda h: h.update(wake_hint="always")),
                    ("sender-without-via", lambda h: h.update(via=None, via_sender="12345")),
                    ("padded-nonce", lambda h: h.update(nonce=h["nonce"] + "="))]:
    item = copy.deepcopy(envelopes[0])
    item["id"], item["expected"] = name, "schema"
    edit(item["header"])
    item["header_cbor_hex"] = cbor(item["header"]).hex()
    envelopes.append(item)
item = copy.deepcopy(envelopes[0])
item["id"], item["expected"] = "nonce-mismatch-signed", "nonce"
item["header"]["nonce"] = b64(bytes(range(1, 13)))
item["header_cbor_hex"] = cbor(item["header"]).hex()
item["signing_hex"] = (b"cowchat/v3/envelope" + cbor(item["header"]) + hashlib.sha256(item["body"].encode()).digest()).hex()
item["signature_hex"] = KEY.sign(bytes.fromhex(item["signing_hex"])).hex()
envelopes.append(item)
item = copy.deepcopy(envelopes[0])
item["id"], item["expected"] = "old-empty-aad-signed", "decrypt"
raw = base64.b64decode(item["body"][5:] + "=" * (-len(item["body"][5:]) % 4))
derived = HKDF(algorithm=hashes.SHA256(), length=32, salt=None, info=b"cowchat-e2e-v1:test-room").derive(secret)
item["body"] = "cow1:" + b64(raw[:12] + ChaCha20Poly1305(derived).encrypt(raw[:12], plain, b""))
item["signing_hex"] = (b"cowchat/v3/envelope" + cbor(item["header"]) + hashlib.sha256(item["body"].encode()).digest()).hex()
item["signature_hex"] = KEY.sign(bytes.fromhex(item["signing_hex"])).hex()
envelopes.append(item)
write("envelopes.json", envelopes)

write("encoding.json", [
    dict(id="mixed-text-key-lengths", input={"aa": 1, "b": 2}, cbor_hex="a261620262616101", expected="accept"),
    dict(id="explicit-null", input={"x": None}, cbor_hex="a16178f6", expected="accept"),
    dict(id="wrong-map-order", input={"aa": 1, "b": 2}, cbor_hex="a262616101616202", expected="encoding"),
    dict(id="duplicate-key", input={"x": 1}, cbor_hex="a2617801617801", expected="encoding"),
    dict(id="nonminimal-integer", input=1, cbor_hex="1801", expected="encoding"),
    dict(id="trailing-data", input=None, cbor_hex="f600", expected="encoding"),
    dict(id="indefinite-array", input=[1], cbor_hex="9f01ff", expected="encoding"),
])

# Stateful replay: a future-edge request remains valid almost 600s from first
# receipt. Eviction at receipt+300s would allow the fourth event through.
events = []
for offset, fresh, expected in [(0, False, "accept"), (300_001, False, "replay"),
                                (599_000, True, "accept"), (600_000, False, "replay"),
                                (600_001, False, "timestamp")]:
    item = request(f"at-{offset}-fresh-{fresh}", stamp=NOW + 300_000, outcome=expected)
    item["now_ms"] = NOW + offset
    if fresh:
        fields = cbor2.loads(bytes.fromhex(item["cbor_hex"]))
        fields[4] = bytes(range(1, 17))
        item["cbor_hex"] = cbor(fields).hex()
        item["signing_hex"] = (b"cowchat/v3/request" + cbor(fields)).hex()
        item["signature_hex"] = KEY.sign(bytes.fromhex(item["signing_hex"])).hex()
    events.append(item)
write("replay-sequence.json", events)

from eth_keys import keys
from eth_utils import keccak

wallet = keys.PrivateKey(bytes([1] * 32))
owner = "0x" + wallet.public_key.to_canonical_address().hex()
identity = dict(v=3, chain_id=1, address=owner, pubkey=PUB, enc_pubkey=bytes(range(32)),
                role="owner", aud="cowchat", gen=1, expires_at=NOW + 100_000)
membership = dict(v=3, chain_id=1, room="test-room", seat=owner, rights=["manage", "read", "write"],
                  door_kind=None, bound_sender=None, from_gen=2, gen=2,
                  signer_kind="wallet", signer_key=None, expires_at=None)
invocation = dict(v=3, chain_id=1, room="test-room", grantee_seat=owner,
                  target_seat=owner + "#builder", scope="wake", budget=((1 << 128) - 1).to_bytes(16, "big"),
                  gen=2, expires_at=None)


def cert(name, kind, fields, signer="wallet"):
    domain = ("cowchat/v3/cert/" + kind).encode()
    raw = cbor(fields)
    signed = domain + raw
    sig = wallet.sign_msg_hash(keccak(signed)).to_bytes() if signer == "wallet" else KEY.sign(signed)
    byte_fields = [k for k, v in fields.items() if isinstance(v, bytes)]
    json_fields = {k: (v.hex() if isinstance(v, bytes) else v) for k, v in fields.items()}
    if isinstance(fields.get("budget"), bytes) and len(fields["budget"]) == 16:
        json_fields["budget"] = str(int.from_bytes(fields["budget"], "big"))
        byte_fields.remove("budget")
    return dict(id=name, kind=kind, fields=json_fields, byte_fields=byte_fields,
                cbor_hex=raw.hex(), signing_hex=signed.hex(), cert_id_hex=hashlib.sha256(signed).hexdigest(),
                signer_kind=signer, public_key_hex=PUB.hex(), signature_hex=sig.hex(),
                expected_owner=owner, expected_chain_id=1, expected_room="test-room",
                expected_gen=fields["gen"], now_ms=NOW, admin_has_manage=True,
                expected="accept")


certs = [cert("identity-owner", "identity", identity),
         cert("membership-wallet", "membership", membership),
         cert("invocation-u128-max", "invocation", invocation)]
for role in ["builder", "actor", "door"]:
    fields = dict(identity, role=role, expires_at=None if role in ["actor", "door"] else NOW + 100_000)
    certs.append(cert("identity-" + role, "identity", fields))
admin = dict(membership, signer_kind="admin", signer_key=PUB)
certs.append(cert("membership-admin", "membership", admin, "admin"))
door = dict(membership, door_kind="telegram", bound_sender="12345", rights=["read", "write"])
certs.append(cert("membership-door", "membership", door))
for name, kind, fields, error in [
    ("empty-rights", "membership", dict(membership, rights=[]), "schema"),
    ("unsorted-rights", "membership", dict(membership, rights=["write", "read"]), "schema"),
    ("unbound-door", "membership", dict(door, bound_sender=None), "schema"),
    ("sender-without-door", "membership", dict(membership, bound_sender="12345"), "schema"),
    ("owner-no-expiry", "identity", dict(identity, expires_at=None), "schema"),
    ("expired-cert", "identity", dict(identity, expires_at=NOW - 1), "expiry"),
    ("membership-expired", "membership", dict(membership, expires_at=NOW - 1), "expiry"),
    ("budget-u64-integer", "invocation", dict(invocation, budget=1), "schema"),
    ("budget-short-bytes", "invocation", dict(invocation, budget=b"\x01"), "schema"),
    ("unknown-cert-field", "identity", dict(identity, future=None), "schema"),
]:
    item = cert(name, kind, fields)
    item["expected"] = error
    certs.append(item)
for name, source, key, value, error in [
    ("wrong-owner", 0, "expected_owner", "0x" + "22" * 20, "authority"),
    ("wrong-chain", 0, "expected_chain_id", 2, "scope"),
    ("wrong-room", 1, "expected_room", "other-room", "scope"),
    ("stale-generation", 1, "expected_gen", 3, "scope"),
    ("cert-id-mismatch", 0, "cert_id_hex", "00" * 32, "cert_id"),
    ("admin-no-manage", 6, "admin_has_manage", False, "authority"),
]:
    item = copy.deepcopy(certs[source])
    item.update(id=name, expected=error)
    item[key] = value
    certs.append(item)
item = copy.deepcopy(certs[0])
item.update(id="wallet-high-s", expected="signature")
sig = bytes.fromhex(item["signature_hex"])
n = int("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141", 16)
item["signature_hex"] = (sig[:32] + (n - int.from_bytes(sig[32:64], "big")).to_bytes(32, "big") + bytes([sig[64] ^ 1])).hex()
certs.append(item)
item = copy.deepcopy(certs[0])
item.update(id="wallet-legacy-v", expected="signature")
sig = bytes.fromhex(item["signature_hex"])
item["signature_hex"] = (sig[:64] + bytes([sig[64] + 27])).hex()
certs.append(item)
write("certificates.json", certs)

from pyhpke import CipherSuite, KEMId, KDFId, AEADId

suite = CipherSuite.new(KEMId.DHKEM_X25519_HKDF_SHA256, KDFId.HKDF_SHA256, AEADId.CHACHA20_POLY1305)
recipient = suite.kem.derive_key_pair(b"cowchat-public-test-recipient")
ephemeral = suite.kem.derive_key_pair(b"cowchat-public-test-ephemeral")
info = b"cowchat/v3/roomkey" + cbor(["test-room", 2])
enc, context = suite.create_sender_context(recipient.public_key, info=info, eks=ephemeral)
ciphertext = context.seal(secret)
wrapped = dict(id="generation-secret", room="test-room", gen=2, info_hex=info.hex(),
               recipient_private_hex=recipient.private_key.to_private_bytes().hex(),
               recipient_public_hex=recipient.public_key.to_public_bytes().hex(),
               enc_hex=enc.hex(), ciphertext_hex=ciphertext.hex(),
               generation_secret_hex=secret.hex(), cbss_released_secret_hex=secret.hex(),
               derived_aead_key_hex=derived.hex(), expected="accept")
wraps = [wrapped]
for name, field, value in [("wrong-room", "room", "other-room"), ("wrong-generation", "gen", 3),
                           ("wrong-recipient", "recipient_private_hex", "01" * 32),
                           ("tampered-wrap", "ciphertext_hex", (bytes([ciphertext[0] ^ 1]) + ciphertext[1:]).hex())]:
    item = copy.deepcopy(wrapped)
    item.update(id=name, expected="decrypt")
    item[field] = value
    wraps.append(item)
write("hpke.json", wraps)

write("manifest.json", dict(
    version=3, status="M0 crypto fixture slice; not full service conformance",
    map_order="RFC8949 core deterministic: bytewise lexicographic encoded keys; spec listed order is documentation only",
    profile="Definite lengths; text map keys; unsigned u64, bytes, text, arrays, maps, bool, null; no tags/floats; unknown signed fields rejected",
    public_test_ed25519_seed_hex=SEED.hex(),
    domains=["cowchat/v3/envelope", "cowchat/v3/request", "cowchat/v3/cert/identity",
             "cowchat/v3/cert/membership", "cowchat/v3/cert/invocation", "cowchat/v3/roomkey"],
    replay=dict(window_ms=300_000, bounds="inclusive", retain="through signed timestamp_ms + window_ms inclusive; checked arithmetic or saturation"),
    source_room="91e48412-213c-445f-84cd-c5bdd1965ff3", ruling_sequences=[18, 21, 23, 26],
    files=[dict(path=name, sha256=hashlib.sha256((ROOT / name).read_bytes()).hexdigest(),
                cases=len(json.loads((ROOT / name).read_text())))
           for name in ["encoding.json", "requests.json", "envelopes.json", "replay-sequence.json", "certificates.json", "hpke.json"]],
    pending=["production verifier/crypto crate and binding parity", "HTTP/WS handlers and raw-target binding",
             "durable nonce cache, message-id dedupe and archive/outbox recovery", "wake HMAC/CloudEvents/filter contracts",
             "certificate/controller/manage-chain authorization against finalized state", "rekey publication and history-cutoff integration",
             "explicit actor-controller commitment mismatch vector at WP1 caller gate",
             "seat-id grammar and door membership manage-right rejection at WP1 service gate",
             "storage adapter and cursor service conformance", "live phase-A route/job/CBSS/debit/settlement evidence"]
))
