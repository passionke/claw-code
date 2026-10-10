import { Alert, AutoComplete, Button, Form, Space, Table, Tag, Tooltip, Typography, message } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  BootstrapCiImageTagsResponse,
  BootstrapPublishJob,
  BootstrapPublishScope,
  BootstrapPublishTemplatesResponse,
  BootstrapTemplateEntry,
  ClusterBootstrapSnapshot,
} from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  /** Wizard only: advance when templates phase is complete. Author: kejiqing */
  onNext?: () => void;
  /**
   * wizard — cluster init step (shows「下一步」).
   * ops — post-init Admin：gateway 不变时用 CI/镜像 tag 重打 e2b 模板（CLI 升级）。
   */
  mode?: "wizard" | "ops";
};

function publishLogLines(job: BootstrapPublishJob | undefined): string[] {
  if (!job) return [];
  const anyJob = job as BootstrapPublishJob & { log_tail?: string[] };
  return anyJob.logTail ?? anyJob.log_tail ?? [];
}

const OBSERVE_KEYS = new Set(["e2bObserve"]);
const WORKER_SET_KEYS = new Set([
  "e2bNasApi",
  "e2bWorker",
  "e2bWorkerRelaxed",
  "e2bWorkerOpencode",
  "e2bWorkerAppserver",
]);

function scopeAffectsKey(scope: BootstrapPublishScope | undefined, key: string): boolean {
  if (!scope) return true; // legacy: treat as all
  if (scope === "observe") return OBSERVE_KEYS.has(key);
  return WORKER_SET_KEYS.has(key);
}

function rowPending(job: BootstrapPublishJob | undefined, key: string): boolean {
  return job?.phase === "running" && scopeAffectsKey(job.scope, key);
}

/**
 * Same product path as bootstrap init: async publish from CI/镜像 tag.
 * Used in wizard and on e2b 平台 for later CLI/template upgrades. Author: kejiqing
 */
export default function TemplateBuildStep({
  snap,
  onRefresh,
  onNext,
  mode = "wizard",
}: Props) {
  const { gatewayBase } = useApp();
  const [form] = Form.useForm<{ imageTag: string }>();
  const [accepting, setAccepting] = useState(false);
  const [tagsLoading, setTagsLoading] = useState(false);
  const [tagOptions, setTagOptions] = useState<{ value: string; label: string }[]>([]);
  const [tagsMeta, setTagsMeta] = useState<BootstrapCiImageTagsResponse | null>(null);
  const [tagsLoadError, setTagsLoadError] = useState<string | null>(null);

  // Observe is decoupled: publish it first from a claw-tap tag, then the worker/others. Author: kejiqing
  const [tapTagForm] = Form.useForm<{ tapImageTag: string }>();
  const [tapTagOptions, setTapTagOptions] = useState<{ value: string; label: string }[]>([]);
  const [tapTagsLoading, setTapTagsLoading] = useState(false);
  const [publishingObserve, setPublishingObserve] = useState(false);

  const job: BootstrapPublishJob | undefined = snap.publishJob;
  const jobRunning = job?.phase === "running";
  const templatesOk =
    !jobRunning && snap.phases.find((p) => p.phase === "e2b_templates")?.complete === true;

  const tableRows: BootstrapTemplateEntry[] = useMemo(() => snap.templateEntries, [snap.templateEntries]);

  const logLines = publishLogLines(job);

  const loadTags = useCallback(async () => {
    if (!gatewayBase) return;
    setTagsLoading(true);
    setTagsLoadError(null);
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
        (current && current.trim()) ||
        data.suggestedTag ||
        snap.suggestedCiImageTag ||
        data.tags[0];
      if (pick) form.setFieldsValue({ imageTag: pick });
    } catch (e) {
      const msg = e instanceof Error ? e.message : "拉取 tag 清单失败";
      setTagsLoadError(msg);
      setTagsMeta(null);
      setTagOptions([]);
      const fallback =
        (form.getFieldValue("imageTag") as string | undefined)?.trim() ||
        snap.suggestedCiImageTag;
      if (fallback) form.setFieldsValue({ imageTag: fallback });
      // Inline Alert already shows failure; do not toast. Author: kejiqing
    } finally {
      setTagsLoading(false);
    }
  }, [form, gatewayBase, snap.suggestedCiImageTag]);

  useEffect(() => {
    void loadTags();
  }, [loadTags]);

  const loadTapTags = useCallback(async () => {
    if (!gatewayBase) return;
    setTapTagsLoading(true);
    try {
      const data = await proxyHttp<BootstrapCiImageTagsResponse>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/ci-image-tags?imageName=claw-tap"
      );
      setTapTagOptions(data.tags.map((t) => ({ value: t, label: t })));
      const current = tapTagForm.getFieldValue("tapImageTag") as string | undefined;
      const pick = (current && current.trim()) || data.suggestedTag || data.tags[0];
      if (pick) tapTagForm.setFieldsValue({ tapImageTag: pick });
    } catch {
      // Keep silent; observe step still allows hand-filled tags. Author: kejiqing
      setTapTagOptions([]);
    } finally {
      setTapTagsLoading(false);
    }
  }, [gatewayBase, tapTagForm]);

  useEffect(() => {
    void loadTapTags();
  }, [loadTapTags]);

  const publishObserve = async () => {
    if (!gatewayBase) return;
    const { tapImageTag } = await tapTagForm.validateFields();
    setPublishingObserve(true);
    try {
      const resp = await proxyHttp<BootstrapPublishTemplatesResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/publish-observe-templates",
        { tapImageTag: tapImageTag.trim() }
      );
      if (resp.accepted) {
        message.success("observe 已受理发布；完成后在「核心组件」重置 observe 生效");
      } else {
        message.warning(resp.message ?? "未受理（可能已有任务在跑）");
      }
      await onRefresh();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "受理失败");
    } finally {
      setPublishingObserve(false);
    }
  };

  // Progress is polled via parent refresh; extra tick while job runs. Author: kejiqing
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
    if (job.phase === "running") {
      const scopeLabel =
        job.scope === "observe" ? "observe" : job.scope === "worker_set" ? "worker 系" : "模板";
      return <Tag color="processing">后台发布中（{scopeLabel}）</Tag>;
    }
    if (job.phase === "succeeded") return <Tag color="success">成功</Tag>;
    return <Tag color="error">失败</Tag>;
  };

  const runningAlertMessage =
    job?.scope === "observe"
      ? "正在发布 observe：仅 observe 行显示发布中；worker 系状态保持不动。"
      : job?.scope === "worker_set"
        ? "正在发布 worker 系模板：observe 行保持不动；本页轮询 status。"
        : "后台异步发布中：本页轮询 status，无需保持长 HTTP。";

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      {mode === "ops" ? (
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          Gateway 镜像不变时，选或手填 CI/镜像 tag（含升级后的{" "}
          <Typography.Text code>claw</Typography.Text>
          ）重打 e2b 核心模板（worker / relaxed / nas-api / opencode / appserver）。与集群
          Init 里「制作模板」同一条 API；新 build 需项目 worker reset 或沙箱失活后才会换上。
        </Typography.Paragraph>
      ) : (
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          契约：<Typography.Text code>POST</Typography.Text> 只受理任务并立刻返回；进度靠{" "}
          <Typography.Text code>GET /bootstrap/status</Typography.Text> 轮询（约 3s），不走同步长连接。
          Tag 可从清单选，也可直接手填。
        </Typography.Paragraph>
      )}

      <Alert
        type="info"
        showIcon
        message="第一步：observe（claw-tap 版本）"
        description="observe 与 worker 解耦，先选 claw-tap 版本单独打 observe；第二步再选 worker tag 打其余模板。"
      />
      <Form form={tapTagForm} layout="inline" style={{ gap: 8 }}>
        <Form.Item
          name="tapImageTag"
          label="claw-tap 版本"
          rules={[{ required: true, message: "请选择或手填 claw-tap tag" }]}
        >
          <AutoComplete
            options={tapTagOptions}
            placeholder={tapTagsLoading ? "加载 claw-tap tag…" : "选择或手填 claw-tap tag"}
            style={{ width: 240 }}
            disabled={jobRunning || publishingObserve}
            filterOption={(input, option) =>
              (option?.value ?? "")
                .toString()
                .toLowerCase()
                .includes(input.trim().toLowerCase())
            }
          />
        </Form.Item>
        <Form.Item>
          <Button
            loading={tapTagsLoading}
            disabled={jobRunning || publishingObserve}
            onClick={() => void loadTapTags()}
          >
            刷新
          </Button>
        </Form.Item>
        <Form.Item>
          <Button
            type="primary"
            loading={publishingObserve}
            disabled={jobRunning}
            onClick={() => void publishObserve()}
          >
            发布 observe
          </Button>
        </Form.Item>
      </Form>

      <Alert
        type="info"
        showIcon
        message="第二步：worker / 其余模板"
        description="选或手填 worker CI/镜像 tag，重打 worker / relaxed / nas-api / opencode / appserver。"
      />

      {tagsMeta ? (
        <Typography.Text type="secondary">
          {tagsMeta.registryHost}/{tagsMeta.repository} · {tagsMeta.tags.length} tags
        </Typography.Text>
      ) : null}

      {tagsLoadError ? (
        <Alert
          type="warning"
          showIcon
          message="无法拉取 tag 清单"
          description={`${tagsLoadError}。可手填已知 tag（如 release-v1.8.15）后发布。`}
        />
      ) : null}

      <Form form={form} layout="inline" style={{ gap: 8 }}>
        <Form.Item
          name="imageTag"
          label="镜像 tag"
          rules={[{ required: true, message: "请选择或手填 tag" }]}
        >
          <AutoComplete
            options={tagOptions}
            placeholder={tagsLoading ? "加载 tag…" : "选择或手填 release / branch tag"}
            style={{ width: 320 }}
            disabled={jobRunning || accepting}
            filterOption={(input, option) =>
              (option?.value ?? "")
                .toString()
                .toLowerCase()
                .includes(input.trim().toLowerCase())
            }
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
            {jobRunning ? "发布中…" : mode === "ops" ? "制作 / 升级模板" : "发布模板"}
          </Button>
        </Form.Item>
        <Form.Item>
          <Button onClick={() => void onRefresh()}>刷新状态</Button>
        </Form.Item>
        {mode === "wizard" && templatesOk && onNext ? (
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
        <Alert type="info" showIcon message={runningAlertMessage} />
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
            render: (v: string | undefined, row: BootstrapTemplateEntry) =>
              rowPending(job, row.key) ? "—" : v ?? "—",
          },
          {
            title: "来源镜像",
            dataIndex: "imageRef",
            ellipsis: { showTitle: false },
            render: (v: string | undefined, row: BootstrapTemplateEntry) =>
              rowPending(job, row.key) || !v ? (
                "—"
              ) : (
                <Tooltip title={v}>
                  <span>{v}</span>
                </Tooltip>
              ),
          },
          {
            title: "镜像 Digest",
            dataIndex: "imageDigest",
            ellipsis: { showTitle: false },
            render: (v: string | undefined, row: BootstrapTemplateEntry) =>
              rowPending(job, row.key) || !v ? (
                "—"
              ) : (
                <Tooltip title={v}>
                  <span>{v.length > 19 ? `${v.slice(0, 19)}…` : v}</span>
                </Tooltip>
              ),
          },
          {
            title: "状态",
            dataIndex: "ready",
            render: (ok: boolean, row: BootstrapTemplateEntry) =>
              rowPending(job, row.key) ? (
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
