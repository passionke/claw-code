/** New project dialog (code + description + optional id). Author: kejiqing */

import { Form, Input, InputNumber, Modal, Typography, message } from "antd";
import { useState } from "react";
import { proxyHttp } from "../api/client";

const CODE_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/;

interface CreateProjectModalProps {
  open: boolean;
  gatewayBase: string;
  onClose: () => void;
  onCreated: (projId: number) => void | Promise<void>;
}

interface CreateProjectForm {
  projectCode: string;
  projectDescription?: string;
  projId?: number;
}

export default function CreateProjectModal({
  open,
  gatewayBase,
  onClose,
  onCreated,
}: CreateProjectModalProps) {
  const [form] = Form.useForm<CreateProjectForm>();
  const [submitting, setSubmitting] = useState(false);

  const handleOk = async () => {
    const values = await form.validateFields();
    const body: {
      projectCode: string;
      projectDescription?: string;
      projId?: number;
    } = {
      projectCode: values.projectCode.trim(),
      projectDescription: values.projectDescription?.trim() || undefined,
    };
    if (values.projId != null && values.projId >= 1) {
      body.projId = values.projId;
    }
    setSubmitting(true);
    try {
      const r = await proxyHttp<{ projId: number }>(
        gatewayBase,
        "POST",
        "/v1/projects",
        body
      );
      message.success(`项目 #${r.projId}（${body.projectCode}）已创建`);
      form.resetFields();
      onClose();
      await onCreated(r.projId);
    } catch (e) {
      message.error(e instanceof Error ? e.message : "创建项目失败");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      title="新建项目"
      open={open}
      okText="创建"
      cancelText="取消"
      confirmLoading={submitting}
      destroyOnClose
      onCancel={() => {
        form.resetFields();
        onClose();
      }}
      onOk={() => void handleOk()}
    >
      <Typography.Paragraph type="secondary" style={{ marginBottom: 16 }}>
        填写项目标识与说明；数字 ID 可留空由系统自动分配。
      </Typography.Paragraph>
      <Form form={form} layout="vertical" requiredMark="optional">
        <Form.Item
          name="projectCode"
          label="项目 Code"
          rules={[
            { required: true, message: "请输入项目 Code" },
            { max: 64, message: "最多 64 个字符" },
            {
              pattern: CODE_PATTERN,
              message: "以字母或数字开头，仅允许字母、数字、-、_",
            },
          ]}
        >
          <Input placeholder="例如 sqlbot-pre" autoFocus />
        </Form.Item>
        <Form.Item
          name="projectDescription"
          label="项目说明"
          rules={[{ max: 500, message: "最多 500 个字符" }]}
        >
          <Input.TextArea rows={3} placeholder="简要描述项目用途（可选）" />
        </Form.Item>
        <Form.Item
          name="projId"
          label="项目 ID"
          tooltip="留空则自动分配下一个可用数字 ID"
          rules={[
            {
              type: "number",
              min: 1,
              message: "ID 须为 ≥ 1 的整数",
            },
          ]}
        >
          <InputNumber style={{ width: "100%" }} placeholder="自动分配" min={1} precision={0} />
        </Form.Item>
      </Form>
    </Modal>
  );
}
