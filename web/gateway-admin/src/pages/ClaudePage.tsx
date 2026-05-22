import { Button, Input, Space, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

const { TextArea } = Input;

export default function ClaudePage() {
  const { gatewayBase, dsId, refreshProjectConfig } = useApp();
  const [content, setContent] = useState("");

  const load = async () => {
    const r = await proxyHttp<{ content?: string; exists?: boolean }>(
      gatewayBase,
      "GET",
      `/v1/project/claude/${dsId}`
    );
    setContent(r.content || "");
    message.info(r.exists ? "CLAUDE.md 已加载" : "文件不存在（空）");
  };

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [gatewayBase, dsId]);

  return (
    <div>
      <Typography.Title level={4}>CLAUDE.md</Typography.Title>
      <Typography.Paragraph type="secondary">
        保存<strong>非空</strong>内容时，将作为<strong>完整系统提示词</strong>（不再前置系统默认段）。
        留空则使用数据库中的系统默认模板 + Rules / Skills 等。恢复默认请到「系统提示词」页。
      </Typography.Paragraph>
      <TextArea rows={18} value={content} onChange={(e) => setContent(e.target.value)} />
      <Space style={{ marginTop: 8 }}>
        <Button
          type="primary"
          onClick={async () => {
            await proxyHttp(gatewayBase, "POST", `/v1/project/claude/${dsId}`, {
              content,
            });
            message.success("CLAUDE.md 已写入临时版");
            await refreshProjectConfig();
          }}
        >
          保存 CLAUDE.md
        </Button>
        <Button onClick={() => load()}>重新加载</Button>
      </Space>
    </div>
  );
}
