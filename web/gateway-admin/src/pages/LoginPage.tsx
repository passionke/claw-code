import { Button, Card, Form, Input, Typography } from "antd";
import { useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { adminLogin, fetchAdminMe } from "../api/client";

/** Post-login path for React Router (basename /admin). Author: kejiqing */
function normalizeAdminNext(raw?: string | null): string {
  let p = (raw ?? "").trim() || "/";
  if (p.startsWith("/admin/")) {
    p = p.slice("/admin".length) || "/";
  } else if (p === "/admin") {
    p = "/";
  }
  if (p.startsWith("/login")) {
    return "/";
  }
  return p.startsWith("/") ? p : `/${p}`;
}

export default function LoginPage() {
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const nav = useNavigate();
  const [params] = useSearchParams();
  const next = normalizeAdminNext(params.get("next"));

  useEffect(() => {
    fetchAdminMe().then((d) => {
      if (d.ok) {
        nav(next, { replace: true });
      }
    });
  }, [nav, next]);

  const onFinish = async (v: { user: string; password: string }) => {
    setErr("");
    setLoading(true);
    try {
      const r = await adminLogin(v.user.trim(), v.password, next);
      if (!r.ok) {
        setErr(r.error || "登录失败");
        return;
      }
      nav(normalizeAdminNext(r.next), { replace: true });
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
          对话页（/admin/chat）无需登录；项目管理需账号密码（服务端 <code>PLAYGROUND_ADMIN_*</code>）
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
        <Link to="/chat">← 返回对话（无需登录）</Link>
      </Card>
    </div>
  );
}
