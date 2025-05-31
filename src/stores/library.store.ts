import { create } from "zustand";

export interface Game {
  remoteId: string;
  title: string;
  iconUrl: string;
  objectId: string;
  shop: "steam";
  winePrefixPath: string | null;
  automaticCloudSync: boolean;
}

interface LibraryStore {
  library: Game[];
  setLibrary: (library: Game[]) => void;
}

export const useLibraryStore = create<LibraryStore>((set) => ({
  library: [],
  setLibrary: (library) =>
    set({ library: library.sort((a, b) => a.title.localeCompare(b.title)) }),
}));
