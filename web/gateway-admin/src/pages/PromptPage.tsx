import { Button, Input, Space, Typography, message } from "antd";
import { useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

const { TextArea } = Input;

export default function PromptPage() {
  const { gatewayBase, dsId } = useApp();
  const [sections, setSections] = useState("");
  const [messageText, setMessageText] = useState("");

  const load = async (reload: boolean) => {
    const method = reload ? "POST" : "GET";
    const r = await proxyHttp<{ sections?: unknown[]; message?: string }>(
      gatewayBase,
      method,
      `/v1/project/prompt/${dsId}/effective`
    );
    setSections(JSON.stringify(r.sections || [], null, 2));
    setMessageText(r.message || "");
    message.success(reload ? "系统提示词已重新加载" : "系统提示词已加载");
  };

  return (
    <div>
      <Typography.Title level={4}>系统提示词（Effective Prompt）</Typography.Title>
      <Space style={{ marginBottom: 8 }}>
        <Button onClick={() => load(false)}>加载</Button>
        <Button onClick={() => load(true)}>重新加载 (POST)</Button>
      </Space>
      <pre style={{ fontSize: 12, maxHeight: 120, overflow: "auto" }}>{sections}</pre>
      <TextArea rows={16} readOnly value={messageText} />
    </div>
  );
}
