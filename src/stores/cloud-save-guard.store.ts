import { create } from "zustand";
import { persist } from "zustand/middleware";

interface CloudSaveGuardStore {
  remoteNewerGames: string[];
  flagRemoteNewer: (objectId: string) => void;
  clearRemoteNewer: (objectId: string) => void;
}

export const useCloudSaveGuard = create<CloudSaveGuardStore>()(
  persist(
    (set) => ({
      remoteNewerGames: [],
      flagRemoteNewer: (objectId) =>
        set((state) =>
          state.remoteNewerGames.includes(objectId)
            ? state
            : { remoteNewerGames: [...state.remoteNewerGames, objectId] }
        ),
      clearRemoteNewer: (objectId) =>
        set((state) => ({
          remoteNewerGames: state.remoteNewerGames.filter(
            (id) => id !== objectId
          ),
        })),
    }),
    { name: "hydra-cloud-save-guard" }
  )
);
