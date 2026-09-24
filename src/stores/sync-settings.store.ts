import { create } from "zustand";
import { persist } from "zustand/middleware";

interface SyncSettingsStore {
  syncBeforePlay: boolean;
  setSyncBeforePlay: (value: boolean) => void;
}

export const useSyncSettings = create<SyncSettingsStore>()(
  persist(
    (set) => ({
      syncBeforePlay: true,
      setSyncBeforePlay: (value) => set({ syncBeforePlay: value }),
    }),
    { name: "hydra-sync-settings" }
  )
);
