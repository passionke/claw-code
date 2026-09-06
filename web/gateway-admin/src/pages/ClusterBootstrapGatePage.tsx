import { Card, ConfigProvider, Typography, theme } from "antd";
import BootstrapWizard from "../components/BootstrapWizard";
import type { ClusterBootstrapSnapshot } from "../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onComplete: () => void;
};

/** Full-screen prerequisite gate before Admin shell. Author: kejiqing */
export default function ClusterBootstrapGatePage({ snap, onRefresh, onComplete }: Props) {
  return (
    <div
      style={{
        minHeight: "100vh",
        overflow: "auto",
        background: "#0f1419",
        padding: "40px 24px",
      }}
    >
      <ConfigProvider theme={{ algorithm: theme.defaultAlgorithm }}>
        <Card
          style={{
            maxWidth: 920,
            margin: "0 auto",
            width: "100%",
            boxShadow: "0 24px 48px rgba(0,0,0,0.35)",
          }}
        >
          <BootstrapWizard snap={snap} onRefresh={onRefresh} onComplete={onComplete} />
        </Card>
      </ConfigProvider>
      <Typography.Paragraph
        style={{ textAlign: "center", marginTop: 16, marginBottom: 0, color: "rgba(255,255,255,0.45)" }}
      >
        Gateway Admin · 首次集群引导
      </Typography.Paragraph>
    </div>
  );
}
