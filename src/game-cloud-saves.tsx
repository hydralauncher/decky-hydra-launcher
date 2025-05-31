import { useCallback, useEffect, useState } from "react";
import { api } from "./hydra-api";
import { callable } from "@decky/api";
import { Button } from "@decky/ui";

const downloadGameArtifact = callable<
  [string, string, string, string, string | null, string],
  void
>("download_game_artifact");

interface GameArtifact {
  id: string;
  artifactLengthInBytes: number;
  downloadOptionTitle: string | null;
  createdAt: string;
  updatedAt: string;
  hostname: string;
  downloadCount: number;
  label?: string;
}

export interface GameCloudSavesProps {
  objectId: string;
  winePrefixPath: string;
}

export function GameCloudSaves({
  objectId,
  winePrefixPath,
}: GameCloudSavesProps) {
  const [artifacts, setArtifacts] = useState<GameArtifact[]>([]);

  useEffect(() => {
    api
      .get<GameArtifact[]>(
        `profile/games/artifacts?objectId=${objectId}&shop=steam`
      )
      .json()
      .then((artifacts) => {
        setArtifacts(artifacts);
      });
  }, []);

  const downloadArtifact = useCallback(async (artifact: GameArtifact) => {
    const response = await api
      .post<{
        downloadUrl: string;
        objectKey: string;
        homeDir: string;
        winePrefixPath: string | null;
      }>(`profile/games/artifacts/${artifact.id}/download`)
      .json();

    console.log(response);

    await downloadGameArtifact(
      objectId,
      response.downloadUrl,
      response.objectKey,
      response.homeDir,
      response.winePrefixPath,
      winePrefixPath
    );
  }, []);

  return (
    <div>
      <h1>{objectId}</h1>

      {artifacts.map((artifact) => (
        <Button key={artifact.id} onClick={() => downloadArtifact(artifact)}>
          {artifact.label}
        </Button>
      ))}
    </div>
  );
}
