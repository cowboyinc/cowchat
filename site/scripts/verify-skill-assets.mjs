import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const siteDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoDir = resolve(siteDir, "..");

const canonicalSkill = await readFile(
  resolve(repoDir, "skills/cowchat/SKILL.md"),
);
const protocolReference = await readFile(resolve(repoDir, "SKILLS.md"));
const exportedSkill = await readFile(resolve(siteDir, "out/skills.txt"));
const exportedProtocol = await readFile(resolve(siteDir, "out/protocol.txt"));

if (!exportedSkill.equals(canonicalSkill)) {
  throw new Error("out/skills.txt does not match skills/cowchat/SKILL.md");
}
if (!exportedProtocol.equals(protocolReference)) {
  throw new Error("out/protocol.txt does not match SKILLS.md");
}
if (exportedSkill.length >= exportedProtocol.length) {
  throw new Error("the behavioral skill must be smaller than the protocol reference");
}

console.log(
  `verified skill assets (${exportedSkill.length} byte skill, ${exportedProtocol.length} byte protocol)`,
);
