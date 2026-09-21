// Independent reference: Node/OpenSSL, not any production Cowchat crypto implementation.
// All keys/nonces here are PUBLIC TEST VALUES and must never be used for real rooms.
import { createCipheriv } from "node:crypto";

const key = Buffer.from(Array.from({ length: 32 }, (_, i) => i));
const nonce = Buffer.from(Array.from({ length: 12 }, (_, i) => i));
const domain = "cowchat-room-message-v1";
function seal(plaintext) {
  const cipher = createCipheriv("chacha20-poly1305", key, nonce, { authTagLength: 16 });
  const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  return "cow1:" + Buffer.concat([nonce, encrypted, cipher.getAuthTag()]).toString("base64").replace(/=+$/, "");
}
const contexts = [
  { room_id: "room-test", key_epoch: "0", message_id: "message-1", text: "Hello 🐎 / \"room\"\nsecond line" },
  { room_id: "room-test", key_epoch: "18446744073709551615", message_id: "message-max", text: "" },
  { room_id: "room-é", key_epoch: "1", message_id: "message-é", text: "Exact UTF-8 scope" },
];
const vectors = contexts.map((context) => {
  const payload = [domain, context.room_id, context.key_epoch, context.message_id, context.text];
  return { ...context, wire: seal(Buffer.from(JSON.stringify(payload))) };
});
const invalid = [
  ["wrong domain", ["wrong-domain", "room-test", "0", "message-1", "text"]],
  ["numeric epoch", [domain, "room-test", 0, "message-1", "text"]],
  ["noncanonical epoch", [domain, "room-test", "00", "message-1", "text"]],
  ["missing text", [domain, "room-test", "0", "message-1"]],
  ["extra field", [domain, "room-test", "0", "message-1", "text", "extra"]],
  ["object payload", { room_id: "room-test", key_epoch: "0", message_id: "message-1", text: "text" }],
].map(([name, payload]) => ({ name, wire: seal(Buffer.from(JSON.stringify(payload))) }));
invalid.push({ name: "invalid UTF-8", wire: seal(Buffer.from([255])) });
invalid.push({ name: "UTF-16 JSON", wire: seal(Buffer.from(JSON.stringify([domain, "room-test", "0", "message-1", "text"]), "utf16le")) });
invalid.push({ name: "lone surrogate", wire: seal(Buffer.from(JSON.stringify([domain, "room-test", "0", "message-1", "\ud800"]))) });
invalid.push({ name: "UTF-8 BOM", wire: seal(Buffer.concat([Buffer.from([239, 187, 191]), Buffer.from(JSON.stringify([domain, "room-test", "0", "message-1", "text"]))])) });
console.log(JSON.stringify({
  profile: domain,
  key_hex: key.toString("hex"),
  nonce_hex: nonce.toString("hex"),
  vectors,
  invalid,
}, null, 2));
