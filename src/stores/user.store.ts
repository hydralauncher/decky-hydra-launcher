import { create } from "zustand";

export interface User {
  id: string;
  username: string;
  displayName: string;
  profileImageUrl: string;
}

interface UserStore {
  user: User | null;
  setUser: (user: User) => void;
}

export const useUserStore = create<UserStore>((set) => ({
  user: null,
  setUser: (user) => set({ user }),
}));
