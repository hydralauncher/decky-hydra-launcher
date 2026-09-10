import { toaster } from "@decky/api";

import { logEvent } from "./events";
import { useCloudSaveGuard, usePlayBlockStore } from "./stores";
import { findGameByShortcutId } from "./pre-play-sync";

/**
 * Blocks the Play button on an app page while a save sync/restore is active.
 *
 * Mechanism: document-level capture listeners (click/touchstart/keydown/
 * keyup — Enter activates on keydown, Space on keyup). No React patching and
 * no cached nodes: matching runs per event via closest() with a composite
 * predicate, so Steam re-renders and client updates cannot stale the handle.
 *
 * Selector strategy (CSS-module prefixes are stable; hash suffixes churn per
 * build): class substring "PlayButton" (required) + button element or
 * role="button" (required) + header proximity (bonus) + visible (required).
 * Pick rule: first visible match in the app header, else first visible match.
 * Tested against Steam client <version — fill in at on-device verification>.
 */
const PLAY_CLASS_FRAGMENT = "PlayButton";
const APP_HEADER_FRAGMENT = "AppDetailsHeader";

export const PLAY_BLOCK_RELEASE_TIMEOUT_MS = 15 * 60 * 1000;
const BLOCKED_TOAST_THROTTLE_MS = 5_000;

const releaseTimers = new Map<string, ReturnType<typeof setTimeout>>();

let activeAppPageId: string | null = null;
export const setActiveAppPage = (appId: string | null) => {
  activeAppPageId = appId;
};

let lastBlockedToast = 0;
let selectorMissLogged = new Set<string>();

const isVisible = (el: HTMLElement) =>
  el.offsetWidth > 0 && el.offsetHeight > 0;

export const findPlayButton = (): HTMLElement | null => {
  const candidates = Array.from(
    document.querySelectorAll<HTMLElement>(`[class*="${PLAY_CLASS_FRAGMENT}"]`)
  );

  const isControl = (el: HTMLElement) =>
    el.tagName === "BUTTON" || el.getAttribute("role") === "button";

  const controls = candidates.filter((el) => isControl(el) && isVisible(el));
  if (controls.length === 0) return null;

  const inHeader = controls.filter((el) =>
    el.closest(`[class*="${APP_HEADER_FRAGMENT}"]`)
  );
  return (inHeader[0] ?? controls[0]) ?? null;
};

const isBlockedPlayTarget = (event: Event): boolean => {
  const store = usePlayBlockStore.getState();
  if (!activeAppPageId || store.blockedGames.size === 0) return false;

  const objectId = activeGameForPage();
  if (!objectId || !store.blockedGames.has(objectId)) return false;

  const target = event.target as HTMLElement | null;
  if (!target) return false;
  const play = findPlayButton();
  return play ? play.contains(target) : false;
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
  const existing = releaseTimers.get(objectId);
  if (existing) clearTimeout(existing);

  usePlayBlockStore.getState().engage(objectId);

  if (!selectorMissLogged.has(objectId)) {
    const candidates = document.querySelectorAll(`[class*="${PLAY_CLASS_FRAGMENT}"]`).length;
    const controls = Array.from(
      document.querySelectorAll<HTMLElement>(`[class*="${PLAY_CLASS_FRAGMENT}"]`)
    ).filter(
      (el) =>
        (el.tagName === "BUTTON" || el.getAttribute("role") === "button") &&
        el.offsetWidth > 0 &&
        el.offsetHeight > 0
    ).length;
    const inHeader = Array.from(
      document.querySelectorAll<HTMLElement>(`[class*="${PLAY_CLASS_FRAGMENT}"]`)
    ).filter((el) => el.closest(`[class*="${APP_HEADER_FRAGMENT}"]`)).length;
    const found = findPlayButton();
    if (!found) {
      selectorMissLogged.add(objectId);
      logEvent(
        `play block: no Play button for ${objectId} (candidates=${candidates} controls=${controls} inHeader=${inHeader})`
      );
    }
  }

  const timer = setTimeout(() => {
    releaseTimers.delete(objectId);
    usePlayBlockStore.getState().disengage(objectId);
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
