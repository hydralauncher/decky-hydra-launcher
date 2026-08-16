import { Focusable, PanelSectionRow } from "@decky/ui";
import { FaPlus, FaTrash } from "react-icons/fa6";
import { GameIcon } from "./game-icon";
import { Button } from "./button";
import { formatBytes } from "../helpers";
import type { Download } from "../api-types";

interface DownloadItemProps {
  download: Download;
  title: string;
  iconUrl?: string | null;
  executablePath?: string | null;
  speed?: number;
  onDismiss?: () => void;
  onAddToSteam?: () => void;
}

export function DownloadItem({ download: d, title, iconUrl, executablePath, speed, onDismiss, onAddToSteam }: DownloadItemProps) {
  const progress = d.extracting ? d.extractionProgress : d.progress;

  const statusLabel = d.status === "complete"
    ? "Complete"
    : d.extracting
      ? `Extracting… ${Math.round(d.extractionProgress * 100)}%`
      : d.status === "active"
        ? `${Math.round(progress * 100)}% · ${formatBytes(d.bytesDownloaded)}${d.fileSize ? ` / ${formatBytes(d.fileSize)}` : ""}${speed ? ` · ${formatBytes(speed)}/s` : ""}`
        : d.status === "waiting"
          ? "Waiting…"
          : d.status === "paused"
            ? "Paused"
            : d.status === "error"
              ? "Error"
              : "";

  const showAddToSteam = d.status === "complete" && !!executablePath && !!onAddToSteam;
  const showDismiss = !!onDismiss;

  return (
    <PanelSectionRow key={d.objectId}>
      <div className="download-item">
        <div className="download-item__header">
          <GameIcon src={iconUrl} alt={title} size={30} />
          <span className="download-item__title">{title}</span>
          {(showAddToSteam || showDismiss) && (
            <Focusable className="download-item__actions" flow-children="horizontal">
              {showAddToSteam && (
                <Button style={{ minWidth: "unset", width: 28, height: 28, padding: 0, display: "flex", alignItems: "center", justifyContent: "center" }} onClick={onAddToSteam} title="Add to Steam">
                  <FaPlus size={12} />
                </Button>
              )}
              {showDismiss && (
                <Button style={{ minWidth: "unset", width: 28, height: 28, padding: 0, display: "flex", alignItems: "center", justifyContent: "center" }} onClick={onDismiss}>
                  <FaTrash size={12} />
                </Button>
              )}
            </Focusable>
          )}
        </div>
        <div className="download-item__progress-bar">
          <div
            className="download-item__progress-fill"
            style={{ width: `${Math.round(progress * 100)}%` }}
          />
        </div>
        <span className="download-item__status">{statusLabel}</span>
      </div>
    </PanelSectionRow>
  );
}
