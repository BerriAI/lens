import { isDeepStrictEqual } from "node:util";
import type { Snapshot, StateStore } from "./state.js";

interface Lease {
  readonly owner: string;
  readonly lease_until: number;
}

export const leaseDuration = 300_000;

export function renewLease<T extends Lease>(
  store: StateStore<T>,
  owner: string,
  signal: AbortSignal,
  now: () => number,
) {
  const controller = new AbortController();
  const active = AbortSignal.any([signal, controller.signal]);
  let queue = Promise.resolve();
  let timer: NodeJS.Timeout | undefined;
  let stopped = false;
  const serial = <R>(work: () => Promise<R>): Promise<R> => {
    const result = queue.then(work);
    queue = result.then(
      () => {},
      () => {},
    );
    return result;
  };
  const owned = async (): Promise<Snapshot<T>> => {
    active.throwIfAborted();
    const current = await store.read();
    if (current.value.owner !== owner || current.value.lease_until <= now())
      throw new Error("Investigation ownership lost");
    return current;
  };
  const renew = async () => {
    try {
      await serial(async () => {
        if (stopped) return;
        const current = await owned();
        await store.commit(current, {
          ...current.value,
          lease_until: now() + leaseDuration,
        });
      });
      if (!stopped) timer = setTimeout(renew, leaseDuration / 5);
    } catch (error) {
      controller.abort(error);
    }
  };
  timer = setTimeout(renew, leaseDuration / 5);
  const leased: StateStore<T> = {
    read: () => serial(owned),
    commit: (previous, value) =>
      serial(async () => {
        const current = await owned();
        if (
          !isDeepStrictEqual(
            { ...current.value, lease_until: previous.value.lease_until },
            previous.value,
          )
        )
          throw new Error("Investigation state changed");
        return store.commit(current, {
          ...value,
          lease_until: value.lease_until === 0 ? 0 : current.value.lease_until,
        });
      }),
  };
  return {
    store: leased,
    signal: active,
    async stop() {
      stopped = true;
      clearTimeout(timer);
      await queue;
    },
  };
}
