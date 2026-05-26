import { Button, Input, Space, Typography } from "antd";
import type { VersionEntry } from "../types/project";

const DRAFT_REV = "__draft__";

type Props = {
  record: VersionEntry;
  /** Pending note for draft row (before commit). */
  draftNote?: string;
  editingRev: string | null;
  editValue: string;
  onStartEdit: (rev: string, initial: string) => void;
  onEditChange: (value: string) => void;
  onCancelEdit: () => void;
  onSave: (record: VersionEntry, value: string) => void | Promise<void>;
};

/** Double-click to edit; save button on the right. Author: kejiqing */
export default function VersionNoteCell({
  record,
  draftNote = "",
  editingRev,
  editValue,
  onStartEdit,
  onEditChange,
  onCancelEdit,
  onSave,
}: Props) {
  const rev = record.isDraft ? DRAFT_REV : record.contentRev;
  const editing = editingRev === rev;
  const display = record.isDraft
    ? draftNote.trim() || "—"
    : record.note?.trim() || "—";

  if (editing) {
    return (
      <Space.Compact style={{ width: "100%", maxWidth: 280 }}>
        <Input
          size="small"
          value={editValue}
          onChange={(e) => onEditChange(e.target.value)}
          placeholder="备注"
          autoFocus
          onPressEnter={() => onSave(record, editValue)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onCancelEdit();
          }}
        />
        <Button
          type="primary"
          size="small"
          onClick={() => onSave(record, editValue)}
        >
          保存
        </Button>
      </Space.Compact>
    );
  }

  return (
    <Typography.Text
      style={{ cursor: "pointer", userSelect: "none" }}
      onDoubleClick={() =>
        onStartEdit(rev, record.isDraft ? draftNote : record.note || "")
      }
      title="双击编辑备注"
    >
      {display}
    </Typography.Text>
  );
}
