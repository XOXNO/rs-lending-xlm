import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { rpc, xdr } from '@stellar/stellar-sdk';
const [networksPath, destination] = process.argv.slice(2);
const networks = JSON.parse(readFileSync(networksPath));
const result = {};
const settings = ['ContractComputeV0','ContractLedgerCostV0','ContractEventsV0','ContractBandwidthV0'];
for (const name of ['testnet','mainnet']) {
  const server = new rpc.Server(networks[name].rpc_url);
  const keys = settings.map(setting => xdr.LedgerKey.configSetting(new xdr.LedgerKeyConfigSetting({configSettingId:xdr.ConfigSettingId[`configSetting${setting}`]()})));
  const entries = await server.getLedgerEntries(...keys);
  assert.equal(entries.entries.length, settings.length, `missing ${name} resource configuration`);
  const limits = {};
  for (const {val} of entries.entries) {
    const setting = val.configSetting().value();
    for (const [key, value] of Object.entries(setting._attributes)) limits[key] = value.toString();
  }
  result[name] = {limits, ledger:entries.latestLedger, network:await server.getNetwork(), version:await server.getVersionInfo()};
}
writeFileSync(destination, JSON.stringify(result,null,2));
