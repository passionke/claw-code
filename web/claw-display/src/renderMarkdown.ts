import mermaid from "mermaid";

let mermaidReady = false;

function ensureMermaid(): void {
  if (mermaidReady) return;
  mermaid.initialize({
    startOnLoad: false,
    theme: "dark",
    securityLevel: "strict",
  });
  mermaidReady = true;
}

export type WorkspaceMediaUrl = (relativePath: string) => string | null;

/** Resolve workspace / relative image paths for Markdown img tags. Author: kejiqing */
export function rewriteMarkdownMediaUrls(
  root: HTMLElement,
  workspaceMediaUrl?: WorkspaceMediaUrl,
): void {
  if (!workspaceMediaUrl) return;
  for (const img of root.querySelectorAll("img")) {
    const raw = img.getAttribute("src")?.trim() ?? "";
    if (!raw || raw.startsWith("data:") || /^https?:\/\//i.test(raw)) continue;
    let rel = raw;
    if (rel.startsWith("workspace:")) {
      rel = rel.slice("workspace:".length);
    }
    rel = rel.replace(/^\.\//, "").replace(/^\/+/, "");
    const url = workspaceMediaUrl(rel);
    if (url) img.setAttribute("src", url);
  }
}

/** Render ```mermaid fences after marked HTML. Author: kejiqing */
export async function renderMermaidBlocks(root: HTMLElement): Promise<void> {
  const blocks = root.querySelectorAll("pre > code.language-mermaid");
  if (!blocks.length) return;
  ensureMermaid();
  let idx = 0;
  for (const code of blocks) {
    const pre = code.parentElement;
    if (!pre) continue;
    const source = code.textContent?.trim() ?? "";
    const host = document.createElement("div");
    host.className = "claw-mermaid";
    if (!source) {
      pre.replaceWith(host);
      continue;
    }
    const id = `claw-mmd-${Date.now()}-${idx++}`;
    try {
      const { svg } = await mermaid.render(id, source);
      host.innerHTML = svg;
    } catch {
      host.classList.add("claw-mermaid-error");
      host.textContent = source;
    }
    pre.replaceWith(host);
  }
}

export async function enhanceAssistantProse(
  root: HTMLElement,
  workspaceMediaUrl?: WorkspaceMediaUrl,
): Promise<void> {
  rewriteMarkdownMediaUrls(root, workspaceMediaUrl);
  await renderMermaidBlocks(root);
}
