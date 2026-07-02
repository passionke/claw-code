// Claw VS Code Chat participant — stub LM + CDP over gateway agent WS. Author: kejiqing
const vscode = require("vscode");

const LM_VENDOR = "claw.lm";
const STUB_MODEL = {
  id: "claw-stub",
  name: "Claw Stub",
  family: "claw",
  version: "1.0.0",
  maxInputTokens: 8192,
  maxOutputTokens: 8192,
  capabilities: {},
  isDefault: true,
};

const OSC_PREFIX = "\x1b]1337;Claw;";
const OSC_SUFFIX = "\x07";

/** @param {string} encoded */
function decodeBase64Url(encoded) {
  if (typeof Buffer !== "undefined") {
    return Buffer.from(encoded, "base64url").toString("utf8");
  }
  const b64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const pad = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  return decodeURIComponent(
    Array.from(atob(pad), (c) => "%" + c.charCodeAt(0).toString(16).padStart(2, "0")).join("")
  );
}

/** @param {string} input */
function extractCdpFrames(input) {
  /** @type {object[]} */
  const frames = [];
  let clean = "";
  let rest = input;
  while (true) {
    const start = rest.indexOf(OSC_PREFIX);
    if (start < 0) {
      clean += rest;
      break;
    }
    clean += rest.slice(0, start);
    rest = rest.slice(start + OSC_PREFIX.length);
    const end = rest.indexOf(OSC_SUFFIX);
    if (end < 0) {
      break;
    }
    const encoded = rest.slice(0, end);
    rest = rest.slice(end + OSC_SUFFIX.length);
    try {
      const json = decodeBase64Url(encoded);
      frames.push(JSON.parse(json));
    } catch {
      /* ignore malformed */
    }
  }
  return { frames, clean };
}

/** @param {import("vscode").ChatResponseStream} stream @param {object} ev */
function applyCdpEvent(stream, ev) {
  const kind = ev && ev.ev;
  if (!kind) return;
  switch (kind) {
    case "content.delta":
      if (typeof ev.text === "string" && ev.text) {
        stream.markdown(ev.text);
      }
      break;
    case "transcript.note":
      if (typeof ev.text === "string" && ev.text) {
        stream.markdown(`\n> ${ev.text}\n`);
      }
      break;
    case "tool.call":
      stream.markdown(
        `\n**tool** \`${ev.name || "?"}\`${ev.summary ? `: ${ev.summary}` : ""}\n`
      );
      break;
    case "status":
      if (ev.phase === "failed" && ev.label) {
        stream.markdown(`\n*${ev.label}*\n`);
      }
      break;
    default:
      break;
  }
}

/** @param {unknown} value @returns {number | null} */
function positiveInt(value) {
  if (typeof value === "number" && Number.isInteger(value) && value > 0) {
    return value;
  }
  if (typeof value === "string" && value.trim()) {
    const n = parseInt(value.trim(), 10);
    if (n > 0) return n;
  }
  return null;
}

/** @param {vscode.Uri} settingsUri */
async function readProjIdFromSettingsFile(settingsUri) {
  try {
    const raw = await vscode.workspace.fs.readFile(settingsUri);
    const text =
      typeof Buffer !== "undefined"
        ? Buffer.from(raw).toString("utf8")
        : new TextDecoder().decode(raw);
    const cfg = JSON.parse(text);
    return positiveInt(cfg["claw.projId"]);
  } catch {
    return null;
  }
}

/** @param {vscode.WorkspaceFolder} folder */
async function resolveProjIdInFolder(folder) {
  const scoped = vscode.workspace.getConfiguration("claw", folder.uri);
  const fromCfg = positiveInt(scoped.get("projId"));
  if (fromCfg) return fromCfg;
  const paths = [
    vscode.Uri.joinPath(folder.uri, ".vscode", "settings.json"),
    vscode.Uri.joinPath(folder.uri, "home", ".vscode", "settings.json"),
  ];
  for (const p of paths) {
    const id = await readProjIdFromSettingsFile(p);
    if (id) return id;
  }
  return null;
}

/**
 * When OVS opens `/home/workspace` (default), scan `proj_N/home/.vscode/settings.json`.
 * Only auto-picks when exactly one project has `claw.projId` (no folder-name guessing).
 * @param {vscode.WorkspaceFolder} root
 * @returns {Promise<number | null>}
 */
async function resolveProjIdFromProjTree(root) {
  /** @type {number[]} */
  const found = [];
  try {
    const entries = await vscode.workspace.fs.readDirectory(root.uri);
    for (const [name, type] of entries) {
      if (type !== vscode.FileType.Directory || !/^proj_\d+$/.test(name)) continue;
      const id = await readProjIdFromSettingsFile(
        vscode.Uri.joinPath(root.uri, name, "home", ".vscode", "settings.json")
      );
      if (id) found.push(id);
    }
  } catch {
    return null;
  }
  if (found.length === 1) return found[0];
  return null;
}

const DEFAULT_GATEWAY_HOST = "gateway-rs:8080";

/** @returns {string} */
function resolveGatewayHost(cfg) {
  const fromEnv = typeof process !== "undefined" && process.env && process.env.CLAW_GATEWAY_HOST;
  if (fromEnv && String(fromEnv).trim()) {
    return String(fromEnv).trim();
  }
  const fromCfg = String(cfg.get("gatewayHost") ?? "").trim();
  if (fromCfg) {
    return fromCfg;
  }
  const inspected = cfg.inspect("gatewayHost");
  const fromInspect = String(
    inspected?.workspaceFolderValue ??
      inspected?.workspaceValue ??
      inspected?.machineValue ??
      inspected?.globalValue ??
      ""
  ).trim();
  return fromInspect || DEFAULT_GATEWAY_HOST;
}

/**
 * OVS canonical workspace path `.../proj_N/home` — used only to call Gateway materialize,
 * not as projId source of truth (settings.json remains authoritative after materialize).
 * @param {vscode.WorkspaceFolder} folder
 * @returns {number | null}
 */
function ovsWorkspaceProjHint(folder) {
  const p = String(folder.uri.fsPath || folder.uri.path || "").replace(/\\/g, "/");
  const m = p.match(/(?:^|\/)proj_(\d+)\/home\/?$/);
  if (!m) return null;
  const n = parseInt(m[1], 10);
  return n > 0 ? n : null;
}

/** @param {number} projId */
async function materializeOvsWorkspace(projId) {
  const cfg = vscode.workspace.getConfiguration("claw");
  const host = resolveGatewayHost(cfg);
  const url = `http://${host}/v1/projects/${projId}/ovs/workspace`;
  try {
    if (typeof fetch === "function") {
      const res = await fetch(url);
      return res.ok;
    }
    const http = require("http");
    return await new Promise((resolve) => {
      http
        .get(url, (res) => {
          resolve(res.statusCode !== undefined && res.statusCode >= 200 && res.statusCode < 300);
          res.resume();
        })
        .on("error", () => resolve(false));
    });
  } catch {
    return false;
  }
}

/**
 * Project id for gateway WS — explicit config only (no folder-name guessing).
 * Source of truth: `proj_N/home/.vscode/settings.json` (`claw.projId`), written by Gateway
 * (`GET /v1/projects/N/ovs/workspace` or terminal materialize).
 * @returns {Promise<number | null>}
 */
async function resolveProjId() {
  const folders = vscode.workspace.workspaceFolders ?? [];
  for (const folder of folders) {
    const id = await resolveProjIdInFolder(folder);
    if (id) return id;
  }
  for (const folder of folders) {
    const hint = ovsWorkspaceProjHint(folder);
    if (!hint) continue;
    if (await materializeOvsWorkspace(hint)) {
      const id = await resolveProjIdInFolder(folder);
      if (id) return id;
    }
  }
  if (folders.length === 1) {
    const fromTree = await resolveProjIdFromProjTree(folders[0]);
    if (fromTree) return fromTree;
  }

  const cfg = vscode.workspace.getConfiguration("claw");
  const inspected = cfg.inspect("projId");
  const fromWorkspace = positiveInt(inspected?.workspaceValue);
  if (fromWorkspace) return fromWorkspace;
  if (typeof globalThis.location !== "undefined") {
    const q = new URLSearchParams(globalThis.location.search);
    const fromUrl = parseInt(q.get("projId") || q.get("proj_id") || "", 10);
    if (fromUrl > 0) {
      return fromUrl;
    }
  }
  return null;
}

/** @param {number} projId */
function defaultSessionId(projId) {
  return `ovs-${projId}`;
}

/**
 * OVS Chat session key — stable for one Chat panel, new on "New Chat".
 * @param {import("vscode").ChatRequest} request
 * @returns {string}
 */
function resolveOvsChatSessionKey(request) {
  const sid = request && request.sessionId;
  if (typeof sid === "string" && sid.trim()) {
    return sid.trim();
  }
  const res = request && request.sessionResource;
  if (res) {
    if (typeof res.toString === "function") {
      const s = res.toString().trim();
      if (s) return s;
    }
    if (res.fsPath) return String(res.fsPath).trim();
    if (res.path) return String(res.path).trim();
  }
  return "";
}

/** @param {string} chatKey */
function ovsChatSessionSlug(chatKey) {
  const raw = String(chatKey || "").trim();
  if (!raw) return "";
  if (/^[a-zA-Z0-9._-]{1,64}$/.test(raw)) return raw;
  try {
    const crypto = require("crypto");
    return crypto.createHash("sha256").update(raw, "utf8").digest("hex").slice(0, 16);
  } catch {
    return raw
      .replace(/[^a-zA-Z0-9._-]/g, "-")
      .replace(/-+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48);
  }
}

/**
 * Map OVS Chat session → gateway **record** id (`ovs-chat-{projId}-{slug}`).
 * Worker REPL stays `ovs-{projId}`; this id is only for `gateway_turns`.
 * @param {number} projId @param {string} chatKey
 */
function ovsChatRecordSessionId(projId, chatKey) {
  const slug = ovsChatSessionSlug(chatKey);
  if (!slug) return defaultSessionId(projId);
  return `ovs-chat-${projId}-${slug}`;
}

/** @param {number} projId @param {string} [chatSessionId] */
function agentWsParts(projId, chatSessionId) {
  const workerSid = defaultSessionId(projId);
  const recordSid = chatSessionId || workerSid;
  const query = `projId=${projId}&chatSessionId=${encodeURIComponent(recordSid)}`;
  return { workerSid, recordSid, query };
}

/** @param {number} projId @param {string} [chatSessionId] */
function gatewayAgentWsUrl(projId, chatSessionId) {
  const cfg = vscode.workspace.getConfiguration("claw");
  const host = resolveGatewayHost(cfg);
  const { workerSid, query } = agentWsParts(projId, chatSessionId);
  return `ws://${host}/v1/sessions/${encodeURIComponent(workerSid)}/agent/ws?${query}`;
}

/** @param {number} projId @param {string} [chatSessionId] */
function browserGatewayAgentWsUrl(projId, chatSessionId) {
  const cfg = vscode.workspace.getConfiguration("claw");
  const host = String(cfg.get("gatewayPublicHost") ?? "").trim() || "127.0.0.1:8088";
  const { workerSid, query } = agentWsParts(projId, chatSessionId);
  return `ws://${host}/v1/sessions/${encodeURIComponent(workerSid)}/agent/ws?${query}`;
}

/** @param {number} projId @param {string} [chatSessionId] */
function agentWsUrl(projId, chatSessionId) {
  const cfg = vscode.workspace.getConfiguration("claw");
  let base = String(cfg.get("agentWsBase") ?? "").trim();
  if (!base) {
    const inspected = cfg.inspect("agentWsBase");
    base = String(
      inspected?.workspaceFolderValue ??
        inspected?.workspaceValue ??
        inspected?.machineValue ??
        inspected?.globalValue ??
        ""
    ).trim();
  }
  const { workerSid, recordSid, query } = agentWsParts(projId, chatSessionId);
  if (!base && typeof globalThis.location !== "undefined") {
    const loc = globalThis.location;
    const pgPort = String(cfg.get("playgroundPort") ?? "18765").trim();
    // Playground proxies /ovs/agent/ws; direct OVS (:13000) must hit gateway on host.
    if (loc.port === pgPort) {
      const proto = loc.protocol === "https:" ? "wss:" : "ws:";
      return `${proto}//${loc.host}/ovs/agent/ws?${query}&sessionId=${encodeURIComponent(workerSid)}`;
    }
    return browserGatewayAgentWsUrl(projId, recordSid);
  }
  if (base) {
    const sep = base.includes("?") ? "&" : "?";
    return `${base}${sep}${query}&sessionId=${encodeURIComponent(workerSid)}`;
  }
  // Remote EH (Node): no browser cookies — bypass Playground auth, hit gateway directly.
  return gatewayAgentWsUrl(projId, recordSid);
}

/**
 * @param {string} url
 * @param {string} prompt
 * @param {import("vscode").CancellationToken} token
 * @param {import("vscode").ChatResponseStream} stream
 */
function runAgentPrompt(url, prompt, token, stream) {
  return new Promise((resolve, reject) => {
    const WS = globalThis.WebSocket;
    if (!WS) {
      reject(new Error("WebSocket unavailable in this host"));
      return;
    }
    const ws = new WS(url);
    let buf = "";
    let gotPayload = false;
    let settled = false;
    /** @type {ReturnType<typeof setTimeout> | undefined} */
    let doneTimer;

    const finish = () => {
      if (settled) return;
      settled = true;
      if (doneTimer) clearTimeout(doneTimer);
      try {
        ws.close();
      } catch {
        /* ignore */
      }
      resolve();
    };

    const fail = (/** @type {string} */ message) => {
      if (settled) return;
      settled = true;
      if (doneTimer) clearTimeout(doneTimer);
      try {
        ws.close();
      } catch {
        /* ignore */
      }
      reject(new Error(message));
    };

    const onCdp = (/** @type {object} */ ev) => {
      gotPayload = true;
      applyCdpEvent(stream, ev);
      if (ev.ev === "status" && (ev.phase === "done" || ev.phase === "failed")) {
        doneTimer = setTimeout(finish, 200);
      }
    };

    ws.onopen = () => {
      ws.send(JSON.stringify({ type: "prompt", text: prompt }));
    };
    ws.onmessage = (/** @type {{ data: string | ArrayBuffer }} */ evt) => {
      const raw = typeof evt.data === "string" ? evt.data : "";
      if (!raw) return;
      gotPayload = true;
      try {
        const msg = JSON.parse(raw);
        if (msg.type === "cdp" && msg.event) {
          onCdp(msg.event);
          return;
        }
        if (msg.type === "error") {
          stream.markdown(`\n**Error:** ${msg.message || "agent failed"}\n`);
          finish();
          return;
        }
      } catch {
        buf += raw;
        const parsed = extractCdpFrames(buf);
        buf = parsed.clean;
        for (const ev of parsed.frames) {
          onCdp(ev);
        }
      }
    };
    ws.onerror = () => {
      if (!gotPayload) {
        fail("agent WebSocket error (no gateway response — try: gateway.sh pool-reset && gateway.sh up)");
      }
    };
    ws.onclose = (/** @type {{ code?: number; reason?: string }} */ evt) => {
      if (!settled && !gotPayload) {
        const reason = (evt && evt.reason) || "";
        fail(
          reason.trim() ||
            `agent WebSocket closed (${evt && evt.code ? evt.code : "?"}) — pool worker may be busy`
        );
        return;
      }
      finish();
    };
    token.onCancellationRequested(() => {
      try {
        ws.close();
      } catch {
        /* ignore */
      }
      resolve();
    });
  });
}

/** @param {readonly import("vscode").WorkspaceFolder[] | undefined} folders */
function foldersLabel(folders) {
  if (!folders || !folders.length) return "";
  return folders.map((f) => f.uri.fsPath || f.uri.path).join(", ");
}

/** @param {import("vscode").ExtensionContext} context */
function activate(context) {
  const log = vscode.window.createOutputChannel("Claw");
  log.appendLine("activate()");

  // OVS Chat requires a stub LM or UI shows "Language model unavailable". kejiqing
  const lmProvider = vscode.lm.registerLanguageModelChatProvider(LM_VENDOR, {
    provideLanguageModelChatInformation(_options, _token) {
      return [STUB_MODEL];
    },
    provideLanguageModelChatResponse(_model, _messages, _options, _progress, _token) {
      return Promise.resolve();
    },
    provideTokenCount(_model, _text, _token) {
      return Promise.resolve(1);
    },
  });
  log.appendLine("registerLanguageModelChatProvider ok");

  const participant = vscode.chat.createChatParticipant(
    "claw.claw",
    async (request, _chatContext, stream, token) => {
      const text = (request.prompt || "").trim();
      log.appendLine(`handler prompt=${JSON.stringify(text)}`);
      const projId = await resolveProjId();
      if (!projId) {
        const opened =
          foldersLabel(vscode.workspace.workspaceFolders) ||
          "(no workspace folder)";
        stream.markdown(
          "**Claw:** `claw.projId` not set.\n\n" +
            `- Opened: \`${opened}\`\n` +
            "- Gateway writes it via `GET /v1/projects/{id}/ovs/workspace` into `proj_N/home/.vscode/settings.json`.\n" +
            "- **Use Playground entry** (materialize + redirect): `http://127.0.0.1:18765/ovs?projId=N`\n" +
            "- Or: `curl http://127.0.0.1:8088/v1/projects/N/ovs/workspace` then reopen `proj_N/home`."
        );
        return { metadata: { command: "" } };
      }
      const chatKey = resolveOvsChatSessionKey(request);
      const recordSessionId = ovsChatRecordSessionId(projId, chatKey);
      const url = agentWsUrl(projId, recordSessionId);
      if (!url) {
        stream.markdown("Claw agent WS URL not configured (`claw.agentWsBase`).");
        return { metadata: { command: "" } };
      }
      if (!text) {
        stream.markdown("Empty prompt.");
        return { metadata: { command: "" } };
      }
      log.appendLine(
        `ovs chat key=${JSON.stringify(chatKey)} record=${recordSessionId} worker=ovs-${projId} url=${url}`
      );
      stream.progress("Claw");
      try {
        await runAgentPrompt(url, text.endsWith("\n") ? text : `${text}\n`, token, stream);
      } catch (e) {
        stream.markdown(`\n**Claw error:** ${e.message || e}\n`);
      }
      return { metadata: { command: "" } };
    }
  );
  log.appendLine("createChatParticipant ok");

  participant.iconPath = vscode.Uri.parse(
    "data:image/svg+xml," +
      encodeURIComponent(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#4ec9b0"><circle cx="12" cy="12" r="10"/></svg>'
      )
  );

  context.subscriptions.push(lmProvider, participant, log);
}

function deactivate() {}

module.exports = { activate, deactivate };
