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

export interface GameAssets {
  objectId: string;
  shop: string;
  title: string;
  iconUrl: string;
  libraryHeroImageUrl: string;
  libraryImageUrl: string;
  logoImageUrl: string;
  coverImageUrl: string;
}

export interface Game {
  remoteId: string;
  title: string;
  iconUrl: string;
  objectId: string;
  shop: "steam";
  winePrefixPath: string | null;
  automaticCloudSync: boolean;
  executablePath: string | null;
}

export type DownloadStatus =
  | "active"
  | "waiting"
  | "paused"
  | "error"
  | "complete"
  | "seeding"
  | "extracting";

export interface Download {
  shop: string;
  objectId: string;
  folderName: string | null;
  progress: number;
  bytesDownloaded: number;
  fileSize: number | null;
  status: DownloadStatus | null;
  extracting: boolean;
  extractionProgress: number;
  queued: boolean;
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
