import { toaster } from "@decky/api";
import type { ToastData, ToastNotification } from "@decky/api";
import { Spinner } from "@decky/ui";
import type { CSSProperties } from "react";

import { logEvent } from "./events";

const activeToasts = new Map<string, ToastNotification>();

export const SYNC_TOAST_TIMEOUT_MS = 10_000;

export const showSyncToast = (objectId: string, data: ToastData) => {
  dismissSyncToast(objectId);
  try {
    const notification = toaster.toast(data);
    if (
      notification &&
      typeof (notification as ToastNotification).dismiss === "function"
    ) {
      activeToasts.set(objectId, notification as ToastNotification);
      logEvent(`sync toast shown: ${objectId}`);
    } else {
      logEvent(`sync toast shown without handle: ${objectId}`);
    }
  } catch (error) {
    logEvent(
      `sync toast show failed: ${objectId}: ${error instanceof Error ? error.message : "unknown"}`
    );
    return;
  }
};

export const dismissSyncToast = (objectId: string) => {
  const toast = activeToasts.get(objectId);
  if (!toast) return;
  activeToasts.delete(objectId);
  try {
    toast.dismiss();
    logEvent(`sync toast dismissed: ${objectId}`);
  } catch (error) {
    logEvent(
      `sync toast dismiss failed: ${objectId}: ${error instanceof Error ? error.message : "unknown"}`
    );
  }
};

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "12px",
};

export const SyncToastBody = ({ text }: { text: string }) => (
  <div style={rowStyle}>
    <Spinner />
    <span>{text}</span>
  </div>
);
