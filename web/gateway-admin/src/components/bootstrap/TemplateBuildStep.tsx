import { Alert, Button, Space, Table, Tag, Typography } from "antd";
import type { ClusterBootstrapSnapshot } from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

/** Step 3: host runs build-selfhosted-templates.sh; wizard polls PG status. Author: kejiqing */
export default function TemplateBuildStep({ snap, onRefresh, onNext }: Props) {
  const templatesOk = snap.phases.find((p) => p.phase === "e2b_templates")?.complete;
  const cmd = snap.templateCommands[0];

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        在 <strong>deploy host</strong>（跑 Gateway 的机器）仓库根目录执行下方命令，写入四个核心模板
        buildId。完成后点「刷新状态」。
      </Typography.Paragraph>

      {cmd ? (
        <Alert
          type="info"
          showIcon
          message={cmd.label}
          description={
            <>
              <pre style={{ margin: "8px 0 0", whiteSpace: "pre-wrap" }}>{cmd.command}</pre>
              {cmd.hint ? (
                <Typography.Text type="secondary" style={{ display: "block", marginTop: 8 }}>
                  {cmd.hint}
                </Typography.Text>
              ) : null}
            </>
          }
        />
      ) : null}

      <Space wrap>
        <Button onClick={() => void onRefresh()}>刷新状态</Button>
        {templatesOk ? (
          <Button type="primary" onClick={onNext}>
            下一步
          </Button>
        ) : null}
      </Space>

      <Table
        size="small"
        pagination={false}
        rowKey="key"
        dataSource={snap.templateEntries}
        columns={[
          { title: "键", dataIndex: "key" },
          { title: "Alias", dataIndex: "alias" },
          { title: "buildId", dataIndex: "buildId", render: (v: string | undefined) => v ?? "—" },
          {
            title: "状态",
            dataIndex: "ready",
            render: (ok: boolean) => (
              <Tag color={ok ? "success" : "warning"}>{ok ? "就绪" : "待构建"}</Tag>
            ),
          },
        ]}
      />
    </Space>
  );
}
