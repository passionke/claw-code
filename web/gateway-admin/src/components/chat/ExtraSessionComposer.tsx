import { Input, Space, Typography } from "antd";
import { Link } from "react-router-dom";
import type { ExtraSessionKv } from "../../utils/extraSessionStorage";

export interface ExtraSessionComposerProps {
  fields: string[];
  values: ExtraSessionKv;
  onChange: (next: ExtraSessionKv) => void;
  disabled?: boolean;
}

/** Per-ds extraSession inputs above the chat composer. Author: kejiqing */
export default function ExtraSessionComposer({
  fields,
  values,
  onChange,
  disabled,
}: ExtraSessionComposerProps) {
  if (!fields.length) {
    return (
      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
        未配置 extraSession 字段（
        <Link to="/extra-session">去 extraSession 配置</Link>
        ，需先登录项目管理）
      </Typography.Text>
    );
  }

  return (
    <Space wrap size={[12, 8]} align="start">
      {fields.map((field) => (
        <Space key={field} direction="vertical" size={2}>
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {field}
          </Typography.Text>
          <Input
            value={values[field] ?? ""}
            onChange={(e) => onChange({ ...values, [field]: e.target.value })}
            placeholder=""
            autoComplete="off"
            disabled={disabled}
            style={{ width: 200 }}
          />
        </Space>
      ))}
    </Space>
  );
}
