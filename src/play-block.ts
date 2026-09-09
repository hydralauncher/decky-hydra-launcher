import { toaster } from "@decky/api";

import { logEvent } from "./events";
import { useCloudSaveGuard, usePlayBlockStore } from "./stores";
import { findGameByShortcutId } from "./pre-play-sync";

/**
 * Blocks the Play button on an app page while a save sync/restore is active.
 *
 * Mechanism: document-level capture listeners (click/touchstart/keydown/
 * keyup — Enter activates on keydown, Space on keyup). No React patching:
 * capture survives Steam client updates better and fails open when selectors
 * drift.
 *
 * The selector below was identified on-device; record the tested Steam client
 * version next to it so drift triage is a diff, not rediscovery.
 * Selector: identified on device, Steam client <version> — TODO: fill in.
 */
const PLAY_BUTTON_SELECTOR = '[class*="PlayButton"], [class*="playButton"]';

export const PLAY_BLOCK_RELEASE_TIMEOUT_MS = 15 * 60 * 1000;
const BLOCKED_TOAST_THROTTLE_MS = 5_000;

// Per-game release timers; cleared on every disengage path so a late timeout
// never fires after a clean settle.
const releaseTimers = new Map<string, ReturnType<typeof setTimeout>>();

// Current app page (set by the pre-play route patch). Listeners are
// registered once and read store state + this value per event — never
// captured values, or they go stale.
let activeAppPageId: string | null = null;
export const setActiveAppPage = (appId: string | null) => {
  activeAppPageId = appId;
};

let lastBlockedToast = 0;
let selectorMissLogged = new Set<string>();

const isBlockedPlayTarget = (event: Event): boolean => {
  const store = usePlayBlockStore.getState();
  if (!activeAppPageId || store.blockedGames.size === 0) return false;

  const library = activeGameForPage();
  if (!library || !store.blockedGames.has(library)) return false;

  const target = event.target as HTMLElement | null;
  return Boolean(target?.closest?.(PLAY_BUTTON_SELECTOR));
};

// The app page id is a shortcut appid; the block store keys by objectId.
// Resolution goes through the same mapping the pre-play check uses.
const activeGameForPage = (): string | null =>
  activeAppPageId ? (findGameByShortcutId(activeAppPageId)?.objectId ?? null) : null;

const blockEvent = (event: Event) => {
  if (!isBlockedPlayTarget(event)) return;

  event.preventDefault();
  // Capture-phase stopPropagation does not stop other listeners on the same
  // node; stopImmediatePropagation does.
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
            body: "Open the Hydra plugin → Pending decisions to choose which save to keep.",
          }
        : {
            title: "Save sync in progress",
            body: "Play is available as soon as the cloud save finishes syncing.",
          }
    );
    if (!selectorMissLogged.has(`attempt-${objectId}`)) {
      selectorMissLogged.add(`attempt-${objectId}`);
      logEvent(`play block: blocked attempt on ${objectId}`);
    }
  }
};

export const engagePlayBlock = (objectId: string) => {
  // Re-engage must not leak the previous timer.
  const existing = releaseTimers.get(objectId);
  if (existing) clearTimeout(existing);

  usePlayBlockStore.getState().engage(objectId);

  // Probe once per engage: only log drift when the selector is truly absent,
  // not on every unrelated click.
  if (!document.querySelector(PLAY_BUTTON_SELECTOR) && !selectorMissLogged.has(objectId)) {
    selectorMissLogged.add(objectId);
    logEvent(`play block: selector not found for ${objectId}`);
  }

  const timer = setTimeout(() => {
    releaseTimers.delete(objectId);
    usePlayBlockStore.getState().disengage(objectId);
    // Timeout never cancels the restore; the cloud-save guard stays the
    // authority on what may sync afterwards.
    logEvent(`play block: release timeout for ${objectId}`);
    toaster.toast({
      title: "Save sync is taking long",
      body: "Play is enabled again. Progress still saves to the cloud after you exit.",
    });
  }, PLAY_BLOCK_RELEASE_TIMEOUT_MS);

  releaseTimers.set(objectId, timer);
};

export const disengagePlayBlock = (objectId: string) => {
  const timer = releaseTimers.get(objectId);
  if (timer) {
    clearTimeout(timer);
    releaseTimers.delete(objectId);
  }
  selectorMissLogged.delete(objectId);
  selectorMissLogged.delete(`attempt-${objectId}`);
  usePlayBlockStore.getState().disengage(objectId);
};

let registered = false;

export const registerPlayBlock = () => {
  if (registered) return;
  registered = true;
  for (const type of ["click", "touchstart", "keydown", "keyup"] as const) {
    document.addEventListener(type, blockEvent, { capture: true });
  }
};

export const unregisterPlayBlock = () => {
  if (!registered) return;
  registered = false;
  for (const type of ["click", "touchstart", "keydown", "keyup"] as const) {
    document.removeEventListener(type, blockEvent, { capture: true });
  }
};
