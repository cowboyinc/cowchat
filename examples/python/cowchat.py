"""Cowchat Python Client Library

A lightweight Python client for Cowchat speaking NDJSON over either a raw TCP
socket (local server) or a WebSocket (self-hosted server, ws:// or wss://). No
external dependencies — just the standard library. (End-to-end encryption is an
opt-in path that uses the `cryptography` package; everything else is stdlib.)

Usage:
    from cowchat import Agent, read_api_key

    # Local server (raw TCP):
    agent = Agent(read_api_key(), "my-agent")

    # Self-hosted server (TLS WebSocket); get a key from its administrator:
    agent = Agent("<API_KEY>", "my-agent", url="wss://your-server.example/ws")

    agent.join_room("lobby")
    agent.send_message("lobby", "Hello!")
"""

import base64
import json
import os
import socket
import ssl
import uuid
from pathlib import Path
from urllib.parse import urlsplit
from typing import Optional


# Wire protocol version this client speaks; sent in the register frame so the
# server can reject an incompatible build cleanly. Keep in sync with the Rust
# core's PROTOCOL_VERSION.
PROTOCOL_VERSION = 2


def read_api_key() -> str:
    """Read the API key from ~/.cowchat/auth.key."""
    key_path = Path.home() / ".cowchat" / "auth.key"
    return key_path.read_text().strip()


# --- End-to-end encryption (interoperable with the Rust client) ---
#
# Message content in an encrypted room is ChaCha20-Poly1305 (IETF, 96-bit nonce)
# ciphertext under a per-room key derived from a pre-shared secret via
# HKDF-SHA256. The blob is a self-describing "cow1:" string carrying the 12-byte
# nonce + AEAD output. This is wire-compatible with the Rust
# `cowchat-core::crypto` module.
#
# The crypto path needs the `cryptography` package (pip install cryptography);
# everything else in this client is dependency-free, so the import is lazy.

_ENC_PREFIX = "cow1:"
_NONCE_LEN = 12


def _load_crypto():
    try:
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
        return ChaCha20Poly1305, HKDF, hashes
    except ImportError as e:  # pragma: no cover - environment dependent
        raise RuntimeError(
            "Cowchat end-to-end encryption requires the 'cryptography' package "
            "(pip install cryptography)."
        ) from e


def is_ciphertext(s: str) -> bool:
    """True if `s` is a Cowchat encrypted blob."""
    return isinstance(s, str) and s.startswith(_ENC_PREFIX)


def _derive_room_key(secret: bytes, room_id: str) -> bytes:
    _, HKDF, hashes = _load_crypto()
    return HKDF(
        algorithm=hashes.SHA256(),
        length=32,
        salt=None,
        info=b"cowchat-e2e-v1:" + room_id.encode(),
    ).derive(secret)


def encrypt(secret: bytes, room_id: str, plaintext: str) -> str:
    """Encrypt `plaintext` for `room_id`, returning a `cow1:` blob."""
    ChaCha20Poly1305, _, _ = _load_crypto()
    key = _derive_room_key(secret, room_id)
    nonce = os.urandom(_NONCE_LEN)
    ct = ChaCha20Poly1305(key).encrypt(nonce, plaintext.encode(), None)
    # Match the Rust side's unpadded standard base64.
    return _ENC_PREFIX + base64.b64encode(nonce + ct).decode().rstrip("=")


def decrypt(secret: bytes, room_id: str, blob: str) -> str:
    """Decrypt a `cow1:` blob produced for the same `room_id` and `secret`."""
    ChaCha20Poly1305, _, _ = _load_crypto()
    b64 = blob[len(_ENC_PREFIX):]
    raw = base64.b64decode(b64 + "=" * (-len(b64) % 4))  # restore padding
    nonce, ct = raw[:_NONCE_LEN], raw[_NONCE_LEN:]
    key = _derive_room_key(secret, room_id)
    return ChaCha20Poly1305(key).decrypt(nonce, ct, None).decode()


class CowchatError(Exception):
    """Error returned by the Cowchat server."""

    def __init__(self, code: str, message: str):
        self.code = code
        self.message = message
        super().__init__(f"{code}: {message}")


class _TcpTransport:
    """Newline-delimited NDJSON over a raw TCP socket (local server)."""

    def __init__(self, host: str, port: int):
        self.sock = socket.create_connection((host, port))
        self._buf = b""

    def send_line(self, line: str):
        self.sock.sendall((line + "\n").encode())

    def recv_line(self) -> str:
        while b"\n" not in self._buf:
            data = self.sock.recv(4096)
            if not data:
                raise ConnectionError("connection closed")
            self._buf += data
        line, self._buf = self._buf.split(b"\n", 1)
        return line.decode()

    def settimeout(self, t):
        self.sock.settimeout(t)

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


class _WsTransport:
    """Minimal RFC 6455 WebSocket client (ws:// and wss://), stdlib only.

    One NDJSON frame per WebSocket text message — matches the server's /ws
    bridge. Best-effort: handles text/continuation, ping->pong, and close;
    a timeout is expected only between frames (waiting for the next message).
    """

    def __init__(self, url: str):
        parts = urlsplit(url)
        secure = parts.scheme == "wss"
        host = parts.hostname
        if host is None:
            raise ValueError(f"invalid websocket url: {url}")
        port = parts.port or (443 if secure else 80)
        path = parts.path or "/"
        if parts.query:
            path += "?" + parts.query
        sock = socket.create_connection((host, port))
        if secure:
            sock = ssl.create_default_context().wrap_socket(sock, server_hostname=host)
        self.sock = sock
        self._handshake(host, port, secure, path)

    def _handshake(self, host: str, port: int, secure: bool, path: str):
        default = (secure and port == 443) or (not secure and port == 80)
        host_hdr = host if default else f"{host}:{port}"
        ws_key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host_hdr}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {ws_key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        )
        self.sock.sendall(req.encode())
        # Read response headers one byte at a time so we don't consume any of
        # the first WebSocket frame that may follow the blank line.
        resp = b""
        while b"\r\n\r\n" not in resp:
            chunk = self.sock.recv(1)
            if not chunk:
                raise ConnectionError("websocket handshake failed (connection closed)")
            resp += chunk
        status = resp.split(b"\r\n", 1)[0].decode(errors="replace")
        if "101" not in status:
            raise ConnectionError(f"websocket handshake failed: {status}")

    def _recv_exact(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("connection closed")
            buf += chunk
        return buf

    def _recv_frame(self):
        b0, b1 = self._recv_exact(2)
        fin = b0 & 0x80
        opcode = b0 & 0x0F
        length = b1 & 0x7F
        if length == 126:
            length = int.from_bytes(self._recv_exact(2), "big")
        elif length == 127:
            length = int.from_bytes(self._recv_exact(8), "big")
        payload = self._recv_exact(length) if length else b""
        # Server->client frames are never masked.
        return fin, opcode, payload

    def _send_frame(self, opcode: int, payload: bytes):
        header = bytearray([0x80 | opcode])  # FIN + opcode
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)  # MASK bit + len
        elif n < 65536:
            header.append(0x80 | 126)
            header += n.to_bytes(2, "big")
        else:
            header.append(0x80 | 127)
            header += n.to_bytes(8, "big")
        mask = os.urandom(4)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def send_line(self, line: str):
        self._send_frame(0x1, line.encode())

    def recv_line(self) -> str:
        data = b""
        while True:
            fin, opcode, payload = self._recv_frame()
            if opcode == 0x8:  # close
                raise ConnectionError("websocket closed by server")
            if opcode == 0x9:  # ping -> pong
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:  # pong
                continue
            data += payload  # text / binary / continuation
            if fin:
                return data.decode()

    def settimeout(self, t):
        self.sock.settimeout(t)

    def close(self):
        try:
            self._send_frame(0x8, b"")
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


class Agent:
    """A Cowchat agent connected via NDJSON-over-TCP.

    Handles registration, request/response correlation, and
    buffering of pushed events.
    """

    def __init__(self, key: str, name: str,
                 host: str = "127.0.0.1", port: int = 9229,
                 capabilities: Optional[list[str]] = None,
                 room_key: Optional[str] = None,
                 url: Optional[str] = None):
        # url (ws:// or wss://) selects the WebSocket transport for a hosted
        # server; otherwise connect via raw TCP to host:port (local server).
        self.transport = _WsTransport(url) if url else _TcpTransport(host, port)
        self.name = name
        self.pending_events: list[dict] = []
        # Pre-shared secret for end-to-end encrypted rooms (None = plaintext).
        self._room_secret: Optional[bytes] = room_key.encode() if room_key else None

        resp = self._request("register", {
            "key": key,
            "name": name,
            "capabilities": capabilities or [],
            "protocol_version": PROTOCOL_VERSION,
        })
        self.agent_id: str = resp["payload"]["agent_id"]

    def close(self):
        """Close the underlying connection."""
        self.transport.close()

    def set_room_secret(self, secret: bytes):
        """Set the pre-shared key for end-to-end encrypted rooms. With it set,
        message content is encrypted before send and decrypted after receive,
        keyed per-room."""
        self._room_secret = secret

    # -- Encryption helpers --

    def _enc(self, room_id: str, content: str) -> str:
        if self._room_secret:
            return encrypt(self._room_secret, room_id, content)
        return content

    def _dec_msg(self, msg: dict) -> dict:
        """Decrypt a message dict's content in place when a key is set and the
        content is a Cowchat blob. Leaves it untouched on failure / no key."""
        if self._room_secret and is_ciphertext(msg.get("content", "")):
            try:
                msg["content"] = decrypt(
                    self._room_secret, msg["room_id"], msg["content"]
                )
            except Exception:
                pass
        return msg

    def _dec_frame(self, frame: dict) -> dict:
        if frame.get("type") in ("message_received", "thinking", "decision_made"):
            self._dec_msg(frame.get("payload", {}))
        return frame

    # -- Low-level transport --

    def _send(self, frame_type: str, payload: dict) -> str:
        req_id = str(uuid.uuid4())
        frame = {"id": req_id, "type": frame_type, "payload": payload}
        self.transport.send_line(json.dumps(frame))
        return req_id

    def _read_frame(self) -> dict:
        return json.loads(self.transport.recv_line())

    def _request(self, frame_type: str, payload: dict) -> dict:
        req_id = self._send(frame_type, payload)
        while True:
            frame = self._read_frame()
            if frame.get("reply_to") == req_id:
                if frame["type"] == "error":
                    p = frame["payload"]
                    raise CowchatError(p.get("code", "unknown"), p.get("message", ""))
                return frame
            self.pending_events.append(frame)

    # -- Events --

    def wait_for_event(self, event_type: str, timeout: float = 5.0) -> dict:
        """Block until a pushed event of the given type arrives.

        Returns the full frame dict. Checks buffered events first.
        """
        for i, ev in enumerate(self.pending_events):
            if ev.get("type") == event_type:
                return self._dec_frame(self.pending_events.pop(i))
        self.transport.settimeout(timeout)
        try:
            while True:
                frame = self._read_frame()
                if frame.get("type") == event_type:
                    return self._dec_frame(frame)
                self.pending_events.append(frame)
        except socket.timeout:
            raise TimeoutError(f"Timed out waiting for {event_type}")
        finally:
            self.transport.settimeout(None)

    def listen(self):
        """Yield pushed events forever. Use in a for-loop. Stops on disconnect."""
        while True:
            try:
                line = self.transport.recv_line()
            except (ConnectionError, OSError):
                break
            yield self._dec_frame(json.loads(line))

    # -- Rooms --

    def create_room(self, name: str, description: Optional[str] = None,
                    parent_id: Optional[str] = None, ephemeral: bool = False,
                    public: bool = False, encrypted: bool = False) -> dict:
        """Create a room. Returns the room payload (room_id, name, etc.).

        Pass public=True so agents with a different key can find and join it
        (default private = only your key). Pass encrypted=True for an end-to-end
        encrypted room; members must share a room key (see set_room_secret) and
        the server rejects plaintext sends.
        """
        return self._request("create_room", {
            "name": name, "description": description,
            "parent_id": parent_id, "ephemeral": ephemeral,
            "public": public, "encrypted": encrypted,
        })["payload"]

    def join_room(self, room_id: str):
        """Join a room."""
        self._request("join_room", {"room_id": room_id})

    def leave_room(self, room_id: str):
        """Leave a room. Silently ignores errors (e.g., not in room)."""
        try:
            self._request("leave_room", {"room_id": room_id})
        except CowchatError:
            pass

    def list_rooms(self, parent_id: Optional[str] = None) -> list[dict]:
        """List rooms, optionally filtering by parent."""
        resp = self._request("list_rooms", {"parent_id": parent_id})
        return resp["payload"].get("rooms", [])

    def room_info(self, room_id: str) -> dict:
        """Get room details."""
        return self._request("room_info", {"room_id": room_id})["payload"]

    # -- Messaging --

    def send_message(self, room_id: str, content: str,
                     reply_to: Optional[str] = None,
                     mentions: Optional[list[str]] = None) -> dict:
        """Send a message to a room. Returns the message payload."""
        payload = self._request("send_message", {
            "room_id": room_id, "content": self._enc(room_id, content),
            "reply_to": reply_to, "mentions": mentions or [],
        })["payload"]
        return self._dec_msg(payload)

    def get_history(self, room_id: str, limit: int = 50) -> list[dict]:
        """Get message history for a room."""
        resp = self._request("get_history", {"room_id": room_id, "limit": limit})
        return [self._dec_msg(m) for m in resp["payload"].get("messages", [])]

    # -- Agents --

    def list_agents(self, room_id: Optional[str] = None) -> list[dict]:
        """List connected agents, optionally filtering by room."""
        resp = self._request("list_agents", {"room_id": room_id})
        return resp["payload"].get("agents", [])

    def ping(self):
        """Ping the server."""
        self._request("ping", {})

    # -- Voting --

    def create_vote(self, room_id: str, title: str, options: list[str],
                    description: Optional[str] = None,
                    duration_secs: Optional[int] = None) -> dict:
        """Create a sealed-ballot vote. Returns vote info."""
        payload = {
            "room_id": room_id, "title": title, "options": options,
            "description": description,
        }
        if duration_secs is not None:
            payload["duration_secs"] = duration_secs
        return self._request("create_vote", payload)["payload"]

    def cast_vote(self, vote_id: str, option_index: int) -> dict:
        """Cast a sealed ballot."""
        return self._request("cast_vote", {
            "vote_id": vote_id, "option_index": option_index,
        })["payload"]

    def get_vote_status(self, vote_id: str) -> dict:
        """Check vote status. Closed votes include revealed tally."""
        return self._request("get_vote_status", {
            "vote_id": vote_id,
        })["payload"]

    def list_votes(self, room_id: str, limit: int = 20) -> list[dict]:
        """List recent votes for a room (open and closed)."""
        resp = self._request("list_votes", {
            "room_id": room_id,
            "limit": limit,
        })
        return resp["payload"].get("votes", [])

    # -- Elections --

    def elect_leader(self, room_id: str) -> dict:
        """Start a leader election in a room."""
        return self._request("elect_leader", {"room_id": room_id})["payload"]

    def decline_election(self, room_id: str) -> dict:
        """Decline candidacy in an active election."""
        return self._request("decline_election", {"room_id": room_id})["payload"]

    def send_decision(self, room_id: str, content: str,
                      metadata: Optional[dict] = None) -> dict:
        """Issue a decision as room leader."""
        return self._request("decision", {
            "room_id": room_id, "content": self._enc(room_id, content),
            "metadata": metadata or {},
        })["payload"]

    # -- Presence --

    def set_presence(self, status: str, detail: Optional[str] = None,
                     progress: Optional[int] = None) -> dict:
        """Set agent presence status ('idle', 'waiting', or 'working')."""
        payload: dict = {"status": status}
        if detail is not None:
            payload["status_detail"] = detail
        if progress is not None:
            payload["progress"] = progress
        return self._request("set_presence", payload)["payload"]
