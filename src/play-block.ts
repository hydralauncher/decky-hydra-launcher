import { toaster } from "@decky/api";

import { logEvent } from "./events";
import { dismissSyncToast } from "./sync-toast";
import { useCloudSaveGuard, usePlayBlockStore } from "./stores";
import { findGameByShortcutId, isOperationActive } from "./pre-play-sync";
import { SYNC_OVERLAY_ATTR } from "./sync-block-overlay";

export const PLAY_BLOCK_RELEASE_TIMEOUT_MS = 5 * 60 * 1000;
const BLOCKED_TOAST_THROTTLE_MS = 5_000;
const MAX_BLOCK_EXTENSIONS = 2;

const releaseTimers = new Map<string, ReturnType<typeof setTimeout>>();
const releaseExtensions = new Map<string, number>();

let activeAppPageId: string | null = null;
export const setActiveAppPage = (appId: string | null) => {
  activeAppPageId = appId;
};

let lastBlockedToast = 0;
const blockedAttemptLogged = new Set<string>();

let qamOpen = false;
export const setQamOpen = (open: boolean) => {
  qamOpen = open;
};

const isActivationKey = (event: Event): boolean => {
  if (event.type === "click" || event.type === "touchstart") return true;
  if (event.type === "keydown" || event.type === "keyup") {
    const key = (event as KeyboardEvent).key;
    return key === "Enter" || key === " ";
  }
  return false;
};

const isBlockedPlayTarget = (event: Event): boolean => {
  if (!isActivationKey(event)) return false;
  if (qamOpen) return false;
  const store = usePlayBlockStore.getState();
  if (!activeAppPageId || store.blockedGames.size === 0) return false;

  const objectId = activeGameForPage();
  if (!objectId || !store.blockedGames.has(objectId)) return false;

  const target = event.target as Element | null;
  if (!target || typeof target !== "object") return false;
  if ((target as Node).nodeType !== 1) return false;
  const element = target as Element;
  if (typeof element.closest !== "function") return false;
  if (element.closest(`[${SYNC_OVERLAY_ATTR}]`)) return false;
  return true;
};

const activeGameForPage = (): string | null =>
  activeAppPageId ? (findGameByShortcutId(activeAppPageId)?.objectId ?? null) : null;

const blockEvent = (event: Event) => {
  if (!isBlockedPlayTarget(event)) return;

  event.preventDefault();
  event.stopImmediatePropagation();

  const objectId = activeGameForPage();
  const now = Date.now();
  if (objectId && now - lastBlockedToast > BLOCKED_TOAST_THROTTLE_MS) {
    lastBlockedToast = now;
    const conflicted = useCloudSaveGuard
      .getState()
      .remoteNewerGames.includes(objectId);
    toaster.toast(
      conflicted
        ? {
            title: "Save sync needs a decision",
            body: "Open Hydra → Pending decisions to choose.",
          }
        : {
            title: "Save sync in progress",
            body: "Play unlocks when the sync finishes.",
          }
    );
    if (!blockedAttemptLogged.has(objectId)) {
      blockedAttemptLogged.add(objectId);
      logEvent(`play block: blocked attempt on ${objectId}`);
    }
  }
};

export const engagePlayBlock = (objectId: string) => {
  const existing = releaseTimers.get(objectId);
  if (existing) clearTimeout(existing);
  releaseExtensions.delete(objectId);

  usePlayBlockStore.getState().engage(objectId);
  logEvent(`play block: engaged ${objectId}`);

  const armReleaseTimer = () => {
    const timer = setTimeout(() => {
      releaseTimers.delete(objectId);
      const extensions = releaseExtensions.get(objectId) ?? 0;
      if (isOperationActive(objectId) && extensions < MAX_BLOCK_EXTENSIONS) {
        releaseExtensions.set(objectId, extensions + 1);
        logEvent(`play block: still busy, extending block for ${objectId} (${extensions + 1}/${MAX_BLOCK_EXTENSIONS})`);
        armReleaseTimer();
        return;
      }
      releaseExtensions.delete(objectId);
      disengagePlayBlock(objectId);
      logEvent(`play block: release timeout for ${objectId}`);
      toaster.toast({
        title: "Save sync is taking long",
        body: "Play is on again. Exit still syncs to the cloud.",
      });
    }, PLAY_BLOCK_RELEASE_TIMEOUT_MS);
    releaseTimers.set(objectId, timer);
  };

  armReleaseTimer();
};

export const disengagePlayBlock = (objectId: string) => {
  dismissSyncToast(objectId);
  releaseExtensions.delete(objectId);
  if (!usePlayBlockStore.getState().blockedGames.has(objectId)) return;
  const timer = releaseTimers.get(objectId);
  if (timer) {
    clearTimeout(timer);
    releaseTimers.delete(objectId);
  }
  blockedAttemptLogged.delete(objectId);
  usePlayBlockStore.getState().disengage(objectId);
  logEvent(`play block: disengaged ${objectId}`);
};

let registered = false;

const BLOCKED_EVENT_TYPES = ["click", "touchstart", "keydown", "keyup"] as const;

export const attachBlockListeners = (doc: Document) => {
  for (const type of BLOCKED_EVENT_TYPES) {
    doc.addEventListener(type, blockEvent, { capture: true });
  }
};

export const detachBlockListeners = (doc: Document) => {
  for (const type of BLOCKED_EVENT_TYPES) {
    doc.removeEventListener(type, blockEvent, { capture: true });
  }
};

export const registerPlayBlock = () => {
  if (registered) return;
  registered = true;
  attachBlockListeners(document);
};

export const unregisterPlayBlock = () => {
  if (!registered) return;
  registered = false;
  detachBlockListeners(document);
};
