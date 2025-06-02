import { create } from "zustand";

export interface NavigationStore {
  route: {
    name: string;
    params: Record<string, unknown>;
  } | null;
  setRoute: (route: { name: string; params: Record<string, unknown> }) => void;
}

export const useNavigationStore = create<NavigationStore>((set) => ({
  route: null,
  setRoute: (route) => set({ route }),
}));
