import assert from 'node:assert/strict';

export function receiptAccountId(args, value) {
  const requested = String(args.accountNonce ?? args.accountId ?? 0);
  assert(/^(0|[1-9][0-9]*)$/.test(requested), 'invalid requested account ID');
  const accountId = requested === '0' ? String(value) : requested;
  assert(/^[1-9][0-9]*$/.test(accountId), 'invalid receipt account ID');
  return accountId;
}
