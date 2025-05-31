import { create } from "zustand";

export interface User {
  id: string;
  username: string;
  displayName: string;
  profileImageUrl: string;
  subscription?: {
    expiresAt: string | null;
  };
  quirks: {
    backupsPerGameLimit: number;
  };
}

interface UserStore {
  user: User | null;
  hasActiveSubscription: boolean;
  setUser: (user: User) => void;
}

export const useUserStore = create<UserStore>((set) => ({
  user: null,
  hasActiveSubscription: false,
  setUser: (user) => {
    const expiresAt = new Date(user?.subscription?.expiresAt ?? 0);
    const hasActiveSubscription = expiresAt > new Date();

    set({ user, hasActiveSubscription });
  },
}));
