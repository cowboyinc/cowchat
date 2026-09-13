#!/usr/bin/env python3
"""Public specification fixtures only; no release, encryption, or live key path.

Requires cryptography and cbor2. All Ed25519 seeds are public RFC 8032 test vectors.
The response ciphertext is deliberately synthetic and is NOT an HPKE test vector.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

import cbor2
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "docs/fixtures/cbss-room-session-v1.json"
DOMAIN = "cowchat/cbss-room-session/v1/"


def head(major, value):
    if value < 24:
        return bytes([(major << 5) | value])
    for width, additional in ((1, 24), (2, 25), (4, 26), (8, 27)):
        if value < 1 << (width * 8):
            return bytes([(major << 5) | additional]) + value.to_bytes(width, "big")
    raise ValueError("out of u64 range")


def encode(value):
    if value is None:
        return b"\xf6"
    if isinstance(value, bool):
        return b"\xf5" if value else b"\xf4"
    if isinstance(value, int) and value >= 0:
        return head(0, value)
    if isinstance(value, bytes):
        return head(2, len(value)) + value
    if isinstance(value, str):
        raw = value.encode("utf-8")
        return head(3, len(raw)) + raw
    if isinstance(value, list):
        return head(4, len(value)) + b"".join(encode(item) for item in value)
    raise TypeError(type(value))


def frame(suffix, raw):
    domain = (DOMAIN + suffix).encode("ascii")
    return len(domain).to_bytes(2, "big") + domain + len(raw).to_bytes(4, "big") + raw


def digest(suffix, raw):
    return hashlib.sha256(frame(suffix, raw)).digest()


def public(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def mutate(value):
    if value is None:
        return 1
    if isinstance(value, bool):
        return not value
    if isinstance(value, int):
        return value + 1
    if isinstance(value, bytes):
        return bytes([value[0] ^ 1]) + value[1:]
    if isinstance(value, str):
        return value + "x"
    if isinstance(value, list):
        result = copy.deepcopy(value)
        result[-1] = mutate(result[-1])
        return result
    raise TypeError(type(value))


def build():
    # RFC 8032 section 7.1 TEST 1, TEST 2 and TEST 3. Never provision these keys.
    seeds = [
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
        "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
    ]
    runtime, access, proxy = [Ed25519PrivateKey.from_private_bytes(bytes.fromhex(x)) for x in seeds]
    room = "91e48412-213c-445f-84cd-c5bdd1965ff3"
    seat = "0x" + "22" * 20
    wallet = bytes.fromhex("7e5f4552091a69125d5dfcb7b8c2659029395bdf")
    g2 = bytes.fromhex(
        "93e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334c"
        "f11213945d57e5ac7d055d042b7e024aa2b2f08f0a91260805272dc51051c6e47ad4"
        "fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8"
    )
    assert len(g2) == 96
    b = lambda number: bytes([number]) * 32
    asset = [1, 31337, wallet, b(3), 2, 7, g2]
    aid = digest("asset", encode(asset))
    grant = [1, b(4), 1, asset, room, seat, b(5), wallet, wallet,
             3, 8, 4, 2, 0, b(0), 0, public(runtime), public(access), 1_800_000_000, None]
    gd = digest("grant", encode(grant))
    recipient = bytes.fromhex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    request = [1, gd, b(4), 1, aid, room, seat, b(5), 3, 4, 2, 0, b(0), 0,
               b(6), b(7), b(8), recipient, 1_800_000_010, 1_800_000_040]
    rd = digest("request", encode(request))
    state = [1, 31337, room, seat, b(4), gd, wallet, 2, b(5), 3, 4, 2, 3, b(0), 0, False]
    access_body = [1, rd, gd, b(9), digest("authority-state", encode(state)),
                   2, 5, 1_800_000_011, 1_800_000_040]
    committee = [1, 31337, 9, 1, [[b(10), 1, public(proxy)]], [g2]]
    header = [1, rd, aid, gd, b(6), digest("recipient", recipient), 9,
              digest("committee", encode(committee)), b(10), 1, 1_800_000_040]
    # Only pins framing/signature binding. This is intentionally NOT valid HPKE ciphertext.
    unsigned_response = [encode(header), b(11), bytes(range(64))]
    objects = {"asset": asset, "grant": grant, "request": request,
               "authority-state": state, "access": access_body, "committee": committee,
               "partial-header": header, "partial/response": unsigned_response}
    vectors = {}
    for name, obj in objects.items():
        raw = encode(obj)
        assert raw == cbor2.dumps(obj, canonical=True), name
        assert cbor2.loads(raw) == obj, name
        vectors[name] = {"cbor_hex": raw.hex(), "sha256_framed_hex": digest(name, raw).hex()}
    vectors["recipient"] = {"raw_hex": recipient.hex(), "sha256_framed_hex": digest("recipient", recipient).hex()}
    for role in ("room", "seat", "asset"):
        vectors["grant"][f"{role}_wallet_signing_input_hex"] = frame("grant/" + role, encode(grant)).hex()
    vectors["partial-header"]["hpke_aad_hex"] = frame("partial-aad", encode(header)).hex()
    vectors["partial-header"]["hpke_info_hex"] = (DOMAIN + "partial-hpke").encode("ascii").hex()
    signatures = {}
    mutation_checks = []
    for name, suffix, key, obj in (("request", "request/runtime", runtime, request),
                                   ("access", "access", access, access_body),
                                   ("partial/response", "partial/response", proxy, unsigned_response)):
        signing_input = frame(suffix, encode(obj))
        signature = key.sign(signing_input)
        key.public_key().verify(signature, signing_input)
        signatures[name] = {"domain": DOMAIN + suffix, "public_key_hex": public(key).hex(),
                            "signing_input_hex": signing_input.hex(), "signature_hex": signature.hex()}
        for index in range(len(obj)):
            altered = copy.deepcopy(obj)
            altered[index] = mutate(altered[index])
            try:
                key.public_key().verify(signature, frame(suffix, encode(altered)))
            except InvalidSignature:
                mutation_checks.append({"object": name, "field_index": index,
                                        "expected": "signature-rejected"})
            else:
                raise AssertionError((name, index))
        try:
            key.public_key().verify(signature, frame(suffix + "/wrong-domain", encode(obj)))
        except InvalidSignature:
            mutation_checks.append({"object": name, "mutation": "wrong-domain", "expected": "signature-rejected"})
        else:
            raise AssertionError("domain separation")
    # Every direct header/grant binding changes its independently computed digest.
    for name, obj in (("grant", grant), ("partial-header", header), ("authority-state", state)):
        for index in range(len(obj)):
            altered = copy.deepcopy(obj)
            altered[index] = mutate(altered[index])
            assert digest(name, encode(altered)) != digest(name, encode(obj))
    # One non-minimal encoding: array length 20 encoded using an unnecessary extra byte.
    noncanonical_request = bytes([0x98, len(request)]) + encode(request)[1:]
    assert cbor2.loads(noncanonical_request) == request
    assert encode(cbor2.loads(noncanonical_request)) != noncanonical_request
    return {"spec": "cbss-room-session-v1", "fixture_version": 1,
            "limits": {"request_bytes": 65536, "response_bytes": 4096, "lease_seconds": 30, "proof_age_seconds": 60},
            "notice": "PUBLIC RFC8032 test keys only. Synthetic HPKE response bytes. No wallet signature, HPKE encryption, BLS or authorization-valid proof is claimed.",
            "vectors": vectors, "ed25519": signatures, "binding_negative_cases": mutation_checks,
            "encoding_negative_cases": [{"name": "nonminimal-request-array-length", "cbor_hex": noncanonical_request.hex(), "expected": "noncanonical"}]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify checked-in fixture without writing")
    args = parser.parse_args()
    data = build()
    rendered = json.dumps(data, indent=2) + "\n"
    if args.check:
        if OUTPUT.read_text() != rendered:
            raise SystemExit("fixture differs; review specification before regenerating")
        print(f"public fixtures verified: {len(data['vectors'])} objects, {len(data['binding_negative_cases'])} signed binding rejections; independent CBOR encoding matches")
    else:
        OUTPUT.write_text(rendered)
        print(f"wrote {OUTPUT}")


if __name__ == "__main__":
    main()
