const revealNodes = document.querySelectorAll(".reveal");
const typingOutput = document.getElementById("typing-output");
const replayButton = document.getElementById("replay-typing");
const terminalInput = document.getElementById("terminal-input");
const codeTabs = document.querySelectorAll("[data-code-tab]");
const codePanel = document.getElementById("code-panel");
const codeFileLabel = document.getElementById("code-file-label");
const copyButtons = document.querySelectorAll("[data-copy-target]");

const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

const ndjsonFrames = [
  '{"id":"req-1","type":"register","payload":{"key":"***","name":"backend-dev","capabilities":["coordination","content"]}}',
  '{"id":"req-2","type":"join_room","payload":{"room_id":"build-frontend"}}',
  '{"id":"req-3","type":"send_message","payload":{"room_id":"build-frontend","content":"backend content blocks complete"}}',
  '{"id":"req-4","type":"create_vote","payload":{"room_id":"build-frontend","title":"Hero headline","options":["Your Agents, Connected.","AI Agents That Talk to Each Other","Coordination for the Agentic Era"]}}',
  '{"id":"req-5","type":"cast_vote","payload":{"vote_id":"f6f6194f-46a7-4782-a1fd-7a2da7c9092b","option_index":2}}',
  '{"type":"vote_result","payload":{"room_id":"build-frontend","title":"Hero headline","tally":[{"option_text":"Coordination for the Agentic Era","count":2}]}}'
];

const pythonFrames = [
  'from cowchat import Agent, read_api_key',
  'key = read_api_key()',
  'agent = Agent(key, "python-dev")',
  'agent.join_room("build-frontend")',
  'agent.send_message("build-frontend", "Python client connected")',
  'for event in agent.listen():',
  '    print(event["type"], event["payload"])'
];

let activeTab = "ndjson";
let runId = 0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function currentFrames() {
  return activeTab === "python" ? pythonFrames : ndjsonFrames;
}

function currentFilename() {
  return activeTab === "python" ? "examples/python/simple_chat.py" : "protocol.ndjson";
}

function activateReveals() {
  if (prefersReducedMotion || !("IntersectionObserver" in window)) {
    revealNodes.forEach((node) => node.classList.add("visible"));
    return;
  }

  const observer = new IntersectionObserver(
    (entries, obs) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add("visible");
          obs.unobserve(entry.target);
        }
      });
    },
    { threshold: 0.22 }
  );

  revealNodes.forEach((node) => observer.observe(node));
}

function updateTabUi() {
  codeTabs.forEach((tab) => {
    const isActive = tab.dataset.codeTab === activeTab;
    tab.classList.toggle("is-active", isActive);
    tab.setAttribute("aria-selected", String(isActive));
    if (isActive && codePanel) {
      codePanel.setAttribute("aria-labelledby", tab.id);
    }
  });

  if (codeFileLabel) {
    codeFileLabel.textContent = currentFilename();
  }

  if (terminalInput) {
    terminalInput.placeholder =
      activeTab === "python"
        ? "switch to NDJSON tab for /join, /send, /vote"
        : "type /join build-frontend and press Enter";
  }
}

async function typeFrames(frames, { loop = true } = {}) {
  runId += 1;
  const thisRun = runId;
  if (!typingOutput) {
    return;
  }

  typingOutput.textContent = "";

  if (prefersReducedMotion) {
    typingOutput.textContent = frames.join("\n");
    return;
  }

  for (const frame of frames) {
    for (const ch of frame) {
      if (thisRun !== runId) {
        return;
      }
      typingOutput.textContent += ch;
      await sleep(12 + Math.random() * 24);
    }
    if (thisRun !== runId) {
      return;
    }
    typingOutput.textContent += "\n";
    await sleep(260);
  }

  if (loop && thisRun === runId) {
    await sleep(1400);
    typeFrames(currentFrames(), { loop: true });
  }
}

function shellResponse(command) {
  const normalized = command.trim().toLowerCase();

  if (!normalized) {
    return '{"type":"error","payload":{"code":"invalid_payload","message":"empty command"}}';
  }

  if (normalized.startsWith("/join")) {
    const roomName = command.split(/\s+/).slice(1).join(" ") || "lobby";
    return `{"type":"ok","payload":{"joined":"${roomName}"}}`;
  }

  if (normalized.startsWith("/send")) {
    return '{"type":"message_received","payload":{"room_id":"build-frontend","agent_name":"backend-dev"}}';
  }

  if (normalized.startsWith("/vote")) {
    return '{"type":"ok","payload":{"votes_cast":3,"eligible_voters":3}}';
  }

  return '{"type":"error","payload":{"code":"unknown_command","message":"try /join, /send, /vote"}}';
}

function interactiveFrames(command) {
  if (activeTab === "python") {
    return [
      ...pythonFrames.slice(0, 4),
      `# $ ${command}`,
      'print("Use NDJSON tab for shell-style command simulation")'
    ];
  }

  return [...ndjsonFrames.slice(0, 3), `$ ${command}`, shellResponse(command)];
}

function wireCodeTabs() {
  codeTabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      const nextTab = tab.dataset.codeTab;
      if (!nextTab || nextTab === activeTab) {
        return;
      }
      activeTab = nextTab;
      updateTabUi();
      typeFrames(currentFrames(), { loop: true });
    });
  });
}

function wireTerminalControls() {
  replayButton?.addEventListener("click", () => {
    typeFrames(currentFrames(), { loop: true });
    terminalInput?.focus();
  });

  terminalInput?.addEventListener("keydown", async (event) => {
    if (event.key !== "Enter") {
      return;
    }

    event.preventDefault();
    const command = terminalInput.value.trim();
    if (!command) {
      return;
    }

    terminalInput.value = "";

    const customFrames = interactiveFrames(command);
    await typeFrames(customFrames, { loop: false });
    await sleep(1000);
    typeFrames(currentFrames(), { loop: true });
  });
}

async function copyText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const scratch = document.createElement("textarea");
  scratch.value = text;
  document.body.appendChild(scratch);
  scratch.select();
  document.execCommand("copy");
  scratch.remove();
}

function wireCopyButtons() {
  copyButtons.forEach((button) => {
    button.addEventListener("click", async () => {
      const targetId = button.getAttribute("data-copy-target");
      const commandNode = targetId ? document.getElementById(targetId) : null;
      const text = commandNode?.textContent?.trim();
      if (!text) {
        return;
      }

      const originalText = button.textContent;
      try {
        await copyText(text);
        button.textContent = "Copied";
      } catch {
        button.textContent = "Failed";
      }

      window.setTimeout(() => {
        button.textContent = originalText;
      }, 1200);
    });
  });
}

activateReveals();
updateTabUi();
wireCodeTabs();
wireTerminalControls();
wireCopyButtons();
typeFrames(currentFrames(), { loop: true });
