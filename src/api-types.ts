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

export interface CloudSaveSnapshotSummary {
  id: string;
  version: number;
  createdAt: string;
  updatedAt: string;
  fileCount: number;
  totalSizeBytes: number;
  aggregateHash: string;
}

export interface CloudSaveSyncResult {
  ok: boolean;
  snapshotId: string;
  version: number;
  fileCount: number;
  totalSizeBytes: number;
  uploadedFiles: number;
  skippedFiles: number;
  auth?: Auth;
}

export interface CloudSaveRestoreResult {
  ok: boolean;
  snapshotId: string;
  version: number;
  restoredFiles: number;
  skippedFiles: string[];
  auth?: Auth;
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
