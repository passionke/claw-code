import { useEffect, useState, type ReactNode } from "react";
import { Spin } from "antd";
import { useLocation, useNavigate } from "react-router-dom";
import { fetchAdminMe } from "../api/client";

export default function RequireAuth({ children }: { children: ReactNode }) {
  const nav = useNavigate();
  const loc = useLocation();
  const [ok, setOk] = useState<boolean | null>(null);

  useEffect(() => {
    fetchAdminMe().then((d) => {
      if (d.ok) setOk(true);
      else {
        const next = encodeURIComponent(loc.pathname + loc.search);
        nav(`/login?next=${next}`, { replace: true });
      }
    });
  }, [loc.pathname, loc.search, nav]);

  if (ok !== true) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: 80 }}>
        <Spin size="large" />
      </div>
    );
  }
  return <>{children}</>;
}
