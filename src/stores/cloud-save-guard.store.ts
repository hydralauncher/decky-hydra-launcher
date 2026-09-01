import { create } from "zustand";
import { persist } from "zustand/middleware";

interface CloudSaveGuardStore {
  remoteNewerGames: string[];
  flagRemoteNewer: (objectId: string) => void;
  clearRemoteNewer: (objectId: string) => void;
}

// Games whose remote snapshot is newer than the local sync state. Flagged at
// game launch; post-exit sync is suppressed for them until the user restores
// or syncs manually. Persisted so a plugin reload mid-session does not lose
// the protection.
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
