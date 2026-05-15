type Listener<T> = (payload: T) => void | Promise<void>;

interface EventMap {
  [event: string]: unknown;
}

export class TypedEventBus<Events extends EventMap> {
  private listeners = new Map<keyof Events, Set<Listener<unknown>>>();

  on<K extends keyof Events>(event: K, listener: Listener<Events[K]>): () => void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(listener as Listener<unknown>);
    return () => this.off(event, listener);
  }

  off<K extends keyof Events>(event: K, listener: Listener<Events[K]>): void {
    this.listeners.get(event)?.delete(listener as Listener<unknown>);
  }

  once<K extends keyof Events>(event: K, listener: Listener<Events[K]>): void {
    const wrapper: Listener<Events[K]> = async (payload) => {
      this.off(event, wrapper);
      await listener(payload);
    };
    this.on(event, wrapper);
  }

  async emit<K extends keyof Events>(event: K, payload: Events[K]): Promise<void> {
    const fns = this.listeners.get(event);
    if (!fns) return;
    await Promise.all([...fns].map((fn) => fn(payload)));
  }

  listenerCount<K extends keyof Events>(event: K): number {
    return this.listeners.get(event)?.size ?? 0;
  }
}

// Usage example -------------------------------------------------------
interface AppEvents {
  "user:login":  { userId: string; at: Date };
  "user:logout": { userId: string };
  "order:placed": { orderId: string; total: number };
}

const bus = new TypedEventBus<AppEvents>();

bus.on("user:login", ({ userId, at }) => {
  console.log(`${userId} logged in at ${at.toISOString()}`);
});

bus.once("order:placed", ({ orderId, total }) => {
  console.log(`First order ${orderId}: $${total.toFixed(2)}`);
});
