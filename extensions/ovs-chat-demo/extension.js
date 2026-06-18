// Minimal OVS Chat demo — no gateway, no WS. Author: kejiqing
const vscode = require("vscode");

/** @param {import("vscode").ExtensionContext} context */
function activate(context) {
  const log = vscode.window.createOutputChannel("OVS Chat Demo");
  log.appendLine("activate()");
  const participant = vscode.chat.createChatParticipant(
    "demo.chat",
    (request, _context, stream, _token) => {
      const text = (request.prompt || "").trim() || "(empty)";
      log.appendLine(`handler prompt=${JSON.stringify(text)}`);
      stream.progress("demo ok");
      stream.markdown(`**demo ok**\n\nYou said: \`${text}\`\n`);
      return { metadata: { command: "" } };
    }
  );
  log.appendLine("createChatParticipant ok");
  context.subscriptions.push(participant, log);
}

function deactivate() {}

module.exports = { activate, deactivate };
