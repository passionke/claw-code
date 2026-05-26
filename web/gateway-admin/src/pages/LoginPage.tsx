import { Button, Card, Form, Input, Typography } from "antd";
import { useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { adminLogin } from "../api/client";

export default function LoginPage() {
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const nav = useNavigate();
  const [params] = useSearchParams();
  const next = params.get("next") || "/";

  const onFinish = async (v: { user: string; password: string }) => {
    setErr("");
    setLoading(true);
    try {
      const r = await adminLogin(v.user.trim(), v.password, next);
      if (!r.ok) {
        setErr(r.error || "登录失败");
        return;
      }
      nav(r.next || "/", { replace: true });
    } catch (e) {
      setErr(String((e as Error).message));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 24,
      }}
    >
      <Card style={{ width: 380 }} title="项目管理登录">
        <Typography.Paragraph type="secondary" style={{ marginTop: 0 }}>
          账号密码由服务端 <code>PLAYGROUND_ADMIN_*</code> 配置
        </Typography.Paragraph>
        <Form layout="vertical" onFinish={onFinish} autoComplete="on">
          <Form.Item name="user" label="账号" rules={[{ required: true }]}>
            <Input autoFocus />
          </Form.Item>
          <Form.Item name="password" label="密码" rules={[{ required: true }]}>
            <Input.Password />
          </Form.Item>
          {err ? (
            <Typography.Text type="danger">{err}</Typography.Text>
          ) : null}
          <Form.Item style={{ marginBottom: 8 }}>
            <Button type="primary" htmlType="submit" block loading={loading}>
              登录
            </Button>
          </Form.Item>
        </Form>
        <Typography.Link href="/">← 返回 solve_async 调试</Typography.Link>
      </Card>
    </div>
  );
}
