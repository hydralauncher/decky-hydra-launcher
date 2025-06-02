export interface GameArtifact {
  id: string;
  artifactLengthInBytes: number;
  downloadOptionTitle: string | null;
  createdAt: string;
  updatedAt: string;
  hostname: string;
  downloadCount: number;
  label?: string;
}

export interface Auth {
  accessToken: string;
  refreshToken: string;
  tokenExpirationTimestamp: number;
}

export interface GameStats {
  assets: {
    title: string;
    coverImageUrl: string;
    iconUrl: string;
  };
}

export interface Game {
  remoteId: string;
  title: string;
  iconUrl: string;
  objectId: string;
  shop: "steam";
  winePrefixPath: string | null;
  automaticCloudSync: boolean;
}

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
