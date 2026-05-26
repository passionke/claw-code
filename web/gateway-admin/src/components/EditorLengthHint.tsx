/** Character / line count above config text areas. Author: kejiqing */

import { Typography } from "antd";

export function textEditorStats(text: string): { chars: number; lines: number } {
  const chars = text.length;
  const lines = chars === 0 ? 0 : text.split("\n").length;
  return { chars, lines };
}

export default function EditorLengthHint({
  text,
  label,
}: {
  text: string;
  label?: string;
}) {
  const { chars, lines } = textEditorStats(text);
  const prefix = label ? `${label} · ` : "";
  return (
    <Typography.Text type="secondary" style={{ fontSize: 12, display: "block", marginBottom: 4 }}>
      {prefix}共 {chars.toLocaleString("zh-CN")} 字符 · {lines} 行
    </Typography.Text>
  );
}
