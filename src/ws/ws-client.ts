import { Envelope } from "../generated/envelope";
import { friendRequestEvent } from "./events/friend-request";
import { friendGameSessionEvent } from "./events/friend-game-session";
import { api } from "../hydra-api";
import { isHydraLauncherRunning } from "../events";

export class WSClient {
  private static ws: WebSocket | null = null;
  private static reconnectInterval = 1_000;
  private static readonly maxReconnectInterval = 30_000;
  private static shouldReconnect = true;
  private static reconnecting = false;
  private static heartbeatInterval: number | null = null;

  static async connect() {
    this.shouldReconnect = true;

    try {
      const { token } = await api.post<{ token: string }>("auth/ws").json();

      const wsUrl = `wss://ws.hydralauncher.gg?token=${encodeURIComponent(
        token
      )}`;
      this.ws = new WebSocket(wsUrl);

      this.ws.onopen = () => {
        console.info("WS connected");
        this.reconnectInterval = 1000;
        this.reconnecting = false;
        this.startHeartbeat();
      };

      this.ws.onmessage = async (event) => {
        try {
          if (event.data === "PONG") {
            return;
          }

          const isHydraRunning = await isHydraLauncherRunning();

          const buffer = new Uint8Array(
            event.data instanceof Blob
              ? await event.data.arrayBuffer()
              : event.data
          );
          const envelope = Envelope.fromBinary(buffer);

          console.info("Received WS envelope:", envelope);

          if (
            envelope.payload.oneofKind === "friendRequest" &&
            !isHydraRunning
          ) {
            friendRequestEvent(envelope.payload.friendRequest);
          }

          if (
            envelope.payload.oneofKind === "friendGameSession" &&
            !isHydraRunning
          ) {
            friendGameSessionEvent(envelope.payload.friendGameSession);
          }
        } catch (err) {
          console.error("Failed to parse message:", err);
        }
      };

      this.ws.onclose = () => this.handleDisconnect("close");
      this.ws.onerror = (err) => {
        console.error("WS error:", err);
        this.handleDisconnect("error");
      };
    } catch (err) {
      console.error("Failed to connect WS:", err);
      this.handleDisconnect("auth-failed");
    }
  }

  private static handleDisconnect(reason: string) {
    console.warn(`WS disconnected due to ${reason}`);

    if (this.shouldReconnect) {
      this.cleanupSocket();
      this.tryReconnect();
    }
  }

  private static async tryReconnect() {
    if (this.reconnecting) return;
    this.reconnecting = true;

    console.info(`Reconnecting in ${this.reconnectInterval / 1000}s...`);

    setTimeout(async () => {
      try {
        await this.connect();
      } catch (err) {
        console.error("Reconnect failed:", err);
        this.reconnectInterval = Math.min(
          this.reconnectInterval * 2,
          this.maxReconnectInterval
        );
        this.reconnecting = false;
        this.tryReconnect();
      }
    }, this.reconnectInterval);
  }

  private static cleanupSocket() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }

    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }
  }

  public static close() {
    this.shouldReconnect = false;
    this.reconnecting = false;
    this.cleanupSocket();
  }

  private static startHeartbeat() {
    this.heartbeatInterval = window.setInterval(() => {
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        this.ws.send("PING");
      }
    }, 15_000);
  }
}
