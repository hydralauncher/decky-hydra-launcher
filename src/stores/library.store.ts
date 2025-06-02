import { create } from "zustand";
import type { Game } from "../api-types";

interface LibraryStore {
  library: Game[];
  setLibrary: (library: Game[]) => void;
}

export const useLibraryStore = create<LibraryStore>((set) => ({
  library: [],
  setLibrary: (library) =>
    set({ library: library.sort((a, b) => a.title.localeCompare(b.title)) }),
}));
