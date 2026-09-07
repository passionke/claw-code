import { Alert, Button, Form, Select, Space, Table, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  BootstrapCiImageTagsResponse,
  BootstrapPublishJob,
  BootstrapPublishTemplatesResponse,
  BootstrapTemplateEntry,
  ClusterBootstrapSnapshot,
} from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

function publishLogLines(job: BootstrapPublishJob | undefined): string[] {
  if (!job) return [];
  const anyJob = job as BootstrapPublishJob & { log_tail?: string[] };
  return anyJob.logTail ?? anyJob.log_tail ?? [];
}

/**
 * Step 3 — async publish only:
 *   POST /publish-templates  → accept job (seconds)
 *   GET  /bootstrap/status   → poll phase / logTail / templateEntries
 * Never wait on one long HTTP for e2b Template.build. Author: kejiqing
 */
export default function TemplateBuildStep({ snap, onRefresh, onNext }: Props) {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<{ imageTag: string }>();
  const [accepting, setAccepting] = useState(false);
  const [tagsLoading, setTagsLoading] = useState(false);
  const [tagOptions, setTagOptions] = useState<{ value: string; label: string }[]>([]);
  const [tagsMeta, setTagsMeta] = useState<BootstrapCiImageTagsResponse | null>(null);

  const job: BootstrapPublishJob | undefined = snap.publishJob;
  const jobRunning = job?.phase === "running";
  const templatesOk =
    !jobRunning && snap.phases.find((p) => p.phase === "e2b_templates")?.complete === true;

  const tableRows: BootstrapTemplateEntry[] = useMemo(() => {
    if (!jobRunning) return snap.templateEntries;
    return snap.templateEntries.map((e) => ({
      ...e,
      buildId: undefined,
      ready: false,
    }));
  }, [jobRunning, snap.templateEntries]);

  const logLines = publishLogLines(job);

  const loadTags = useCallback(async () => {
    if (!gatewayBase) return;
    setTagsLoading(true);
    try {
      const data = await proxyHttp<BootstrapCiImageTagsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/ci-image-tags"
      );
      setTagsMeta(data);
      setTagOptions(data.tags.map((t) => ({ value: t, label: t })));
      const current = form.getFieldValue("imageTag") as string | undefined;
      const pick =
        (current && data.tags.includes(current) && current) ||
        data.suggestedTag ||
        snap.suggestedCiImageTag ||
        data.tags[0];
      if (pick) form.setFieldsValue({ imageTag: pick });
    } catch (e) {
      message.error(e instanceof Error ? e.message : "拉取 ACR tag 清单失败");
    } finally {
      setTagsLoading(false);
    }
  }, [form, gatewayBase, snap.suggestedCiImageTag]);

  useEffect(() => {
    void loadTags();
  }, [loadTags]);

  // Progress is polled via useClusterBootstrap (status). Extra tick while job runs. Author: kejiqing
  useEffect(() => {
    if (!jobRunning) return;
    const t = window.setInterval(() => {
      void onRefresh();
    }, 3000);
    return () => clearInterval(t);
  }, [jobRunning, onRefresh]);

  const publish = async () => {
    if (!gatewayBase) return;
    const { imageTag } = await form.validateFields();
    setAccepting(true);
    try {
      // Accept-only: must return before e2b build finishes (gateway tokio::spawn).
      const resp = await proxyHttp<BootstrapPublishTemplatesResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/publish-templates",
        { imageTag: imageTag.trim() }
      );
      if (resp.accepted) {
        message.success("已受理，后台发布中（请看下方轮询日志）");
      } else {
        message.warning(resp.message ?? "未受理（可能已有任务在跑）");
      }
      await onRefresh();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "受理失败");
    } finally {
      setAccepting(false);
    }
  };

  const jobTag = () => {
    if (!job || job.phase === "idle") return <Tag>未发布</Tag>;
    if (job.phase === "running") return <Tag color="processing">后台发布中</Tag>;
    if (job.phase === "succeeded") return <Tag color="success">成功</Tag>;
    return <Tag color="error">失败</Tag>;
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
        契约：<Typography.Text code>POST</Typography.Text> 只受理任务并立刻返回；进度靠{" "}
        <Typography.Text code>GET /bootstrap/status</Typography.Text> 轮询（约 3s），不走同步长连接。
      </Typography.Paragraph>

      {tagsMeta ? (
        <Typography.Text type="secondary">
          {tagsMeta.registryHost}/{tagsMeta.repository} · {tagsMeta.tags.length} tags
        </Typography.Text>
      ) : null}

      <Form form={form} layout="inline" style={{ gap: 8 }}>
        <Form.Item
          name="imageTag"
          label="CI/ACR tag"
          rules={[{ required: true, message: "请从清单选择 tag" }]}
        >
          <Select
            showSearch
            placeholder={tagsLoading ? "加载 tag…" : "选择 release / branch tag"}
            options={tagOptions}
            style={{ width: 320 }}
            disabled={jobRunning || accepting}
            loading={tagsLoading}
            optionFilterProp="label"
            notFoundContent={tagsLoading ? "加载中…" : "无可用 tag"}
          />
        </Form.Item>
        <Form.Item>
          <Button
            loading={tagsLoading}
            disabled={jobRunning || accepting}
            onClick={() => void loadTags()}
          >
            刷新清单
          </Button>
        </Form.Item>
        <Form.Item>
          <Button
            type="primary"
            loading={accepting}
            disabled={jobRunning}
            onClick={() => void publish()}
          >
            {jobRunning ? "发布中…" : "发布模板"}
          </Button>
        </Form.Item>
        <Form.Item>
          <Button onClick={() => void onRefresh()}>刷新状态</Button>
        </Form.Item>
        {templatesOk ? (
          <Form.Item>
            <Button type="primary" onClick={onNext}>
              下一步
            </Button>
          </Form.Item>
        ) : null}
      </Form>

      <Space wrap>
        {jobTag()}
        {job?.imageTag ? <Typography.Text type="secondary">tag={job.imageTag}</Typography.Text> : null}
        {job?.message ? <Typography.Text type="secondary">{job.message}</Typography.Text> : null}
      </Space>

      {job?.phase === "failed" && job.message ? (
        <Alert type="error" showIcon message={job.message} />
      ) : null}

      {jobRunning ? (
        <Alert
          type="info"
          showIcon
          message="后台异步发布中：组件就绪态已清空；本页轮询 status，无需保持长 HTTP。"
        />
      ) : null}

      {logLines.length > 0 ? (
        <Alert
          type="info"
          showIcon
          message="发布日志（status 轮询刷新）"
          description={
            <pre style={{ margin: 0, maxHeight: 280, overflow: "auto", whiteSpace: "pre-wrap" }}>
              {logLines.join("\n")}
            </pre>
          }
        />
      ) : null}

      <Table
        size="small"
        pagination={false}
        rowKey="key"
        dataSource={tableRows}
        columns={[
          { title: "键", dataIndex: "key" },
          { title: "Alias", dataIndex: "alias" },
          {
            title: "buildId",
            dataIndex: "buildId",
            render: (v: string | undefined) => (jobRunning ? "—" : v ?? "—"),
          },
          {
            title: "状态",
            dataIndex: "ready",
            render: (ok: boolean) =>
              jobRunning ? (
                <Tag color="processing">发布中</Tag>
              ) : (
                <Tag color={ok ? "success" : "warning"}>{ok ? "就绪" : "待构建"}</Tag>
              ),
          },
        ]}
      />
    </Space>
  );
}
