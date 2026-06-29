import { wsTokenQuery } from "./auth";
import type { WsMessage } from "../store/protocol";

// WebSocket connection with exponential backoff reconnect. On reconnect we
// refetch /api/state via the caller's onReconnect callback to recover any
// patches missed during the disconnect.

const BASE_MS = 1000;
const MAX_MS = 30_000;

type MessageHandler = (msg: WsMessage) => void;
type StatusHandler = (connected: boolean, attempts: number) => void;

export class WsConnection {
  private ws: WebSocket | null = null;
  private attempts = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly onMessage: MessageHandler;
  private readonly onStatus: StatusHandler;
  private readonly onReconnect: () => void;

  constructor(
    onMessage: MessageHandler,
    onStatus: StatusHandler,
    onReconnect: () => void,
  ) {
    this.onMessage = onMessage;
    this.onStatus = onStatus;
    this.onReconnect = onReconnect;
  }

  connect(): void {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    this.ws = new WebSocket(`${proto}//${location.host}/ws${wsTokenQuery()}`);

    this.ws.onopen = () => {
      const wasReconnecting = this.attempts > 0;
      this.attempts = 0;
      if (wasReconnecting) {
        this.onReconnect();
      }
      this.onStatus(true, 0);
    };

    this.ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data) as WsMessage;
        this.onMessage(data);
      } catch (e) {
        console.error("ws parse error:", e);
      }
    };

    this.ws.onclose = () => {
      this.onStatus(false, this.attempts + 1);
      const delay = Math.min(BASE_MS * 2 ** this.attempts, MAX_MS);
      this.attempts += 1;
      this.timer = setTimeout(() => this.connect(), delay);
    };

    this.ws.onerror = () => {
      // onclose will fire and trigger reconnect.
    };
  }

  close(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }
  }

  get isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  get attemptCount(): number {
    return this.attempts;
  }
}
