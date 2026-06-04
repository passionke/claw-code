import { InfoCircleOutlined } from "@ant-design/icons";
import { Button, Drawer, Typography } from "antd";
import { useState } from "react";

export interface TurnExtraSessionDrawerProps {
  extraSession: Record<string, unknown> | null | undefined;
}

/** View per-turn extraSession snapshot from gateway. Author: kejiqing */
export default function TurnExtraSessionDrawer({ extraSession }: TurnExtraSessionDrawerProps) {
  const [open, setOpen] = useState(false);
  const hasData = extraSession != null && Object.keys(extraSession).length > 0;

  return (
    <>
      <Button
        size="small"
        icon={<InfoCircleOutlined />}
        onClick={() => setOpen(true)}
        disabled={!hasData}
      >
        extraSession
      </Button>
      <Drawer
        title="extraSession"
        open={open}
        onClose={() => setOpen(false)}
        width={480}
        destroyOnClose
      >
        {hasData ? (
          <Typography.Paragraph>
            <pre style={{ margin: 0, whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
              {JSON.stringify(extraSession, null, 2)}
            </pre>
          </Typography.Paragraph>
        ) : (
          <Typography.Text type="secondary">该轮未记录 extraSession</Typography.Text>
        )}
      </Drawer>
    </>
  );
}
