import assert from 'node:assert/strict';
import {setTimeout as delay} from 'node:timers/promises';

const retries = process.env.THROTTLE_RETRIES ?? '6';
assert.match(retries, /^(1?[0-9]|20)$/, 'invalid THROTTLE_RETRIES');

export const throttleBudget = (limit = Number(retries)) => ({used: 0, limit});

export function throttleDelay(attempt, after) {
  const base = Number.isInteger(after) && after > 0 ? Math.min(after, 120) : Math.min(60, 8 * 2 ** (attempt - 1));
  return (base + Math.floor(Math.random() * (Math.floor(base / 2) + 1))) * 1000;
}

export async function whileThrottled(read, budget = throttleBudget(), wait = delay) {
  for (;;) {
    try { return await read(); }
    catch (error) {
      if (error?.response?.status !== 429 || budget.used >= budget.limit) throw error;
      await wait(throttleDelay(++budget.used, error.response.data?.retry_after));
    }
  }
}

export async function postRead(fetcher, url, init, budget = throttleBudget(), wait = delay) {
  for (;;) {
    const response = await fetcher(url, init);
    if (response.status !== 429 || budget.used >= budget.limit) return response;
    const body = await response.json().catch(() => null);
    await wait(throttleDelay(++budget.used, body?.retry_after));
  }
}
