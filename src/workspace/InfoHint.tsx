import { IconButton, Tooltip } from "@orcestr/ui";
import { LuInfo } from "react-icons/lu";

export function InfoHint({
  label,
  content,
}: {
  label: string;
  content: string;
}) {
  return (
    <Tooltip content={content}>
      <IconButton
        icon={<LuInfo size={13} />}
        aria-label={label}
        v="ghost"
        size={1}
        className="context-info"
      />
    </Tooltip>
  );
}
