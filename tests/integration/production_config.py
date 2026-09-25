#!/usr/bin/env python3
"""Production policy with explicit testnet token/provider address substitution."""
import hashlib
import json
from math import isqrt
import re
import sys
from pathlib import Path


def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def load(root, name):
    return json.loads((root / name).read_text())


def canonical(value):
    """CLI encodes i128 as strings; never pass money through float/jq numbers."""
    if isinstance(value, str) and re.fullmatch(r'-?[0-9]+', value):
        return int(value)
    if isinstance(value, list):
        return [canonical(v) for v in value]
    if isinstance(value, dict):
        return {k: canonical(v) for k, v in value.items()}
    return value


def verify_equal(expected, actual):
    # bool is an int subclass: serialized comparison retains that distinction.
    assert json.dumps(canonical(actual), sort_keys=True) == json.dumps(canonical(expected), sort_keys=True), f'policy mismatch: expected {expected}, observed {actual}'


def policy_checks(root, networks):
    markets = load(root, 'markets.json')
    by_name = {m['name']: m for m in markets['markets']}
    checks = []
    def add(contract, method, args, expected):
        argv = []
        for key, value in args.items():
            argv.extend(['--' + key, json.dumps(value, separators=(',', ':'))])
        checks.append(dict(contract=contract, method=method, args=argv, expected=expected))
    for entry in markets.get('references', []) + markets['markets']:
        key = entry.get('key', {'Token': entry.get('asset_address')})
        add('PRICE_AGGREGATOR', 'oracle', dict(key=key), entry['oracle'])
    for market in markets['markets']:
        key = dict(hub_id=market['hub_id'], asset=market['asset_address'])
        expected = dict(market['market_params'], asset_id=market['asset_address'], asset_decimals=market['oracle']['asset_decimals'])
        add('POOL', 'get_sync_data', dict(hub_asset=key), expected)
        checks[-1]['field'] = 'params'
    for config_id, spoke in load(root, 'spokes.json').items():
        spoke_id = networks['testnet']['spoke_ids'][config_id]
        curve = spoke['liquidation_curve']
        add('CONTROLLER', 'get_spoke', dict(spoke_id=spoke_id), dict(
            is_deprecated=False, liquidation_target_hf_wad=curve['target_hf_wad'],
            hf_for_max_bonus_wad=curve['hf_for_max_bonus_wad'],
            liquidation_bonus_factor_bps=curve['liquidation_bonus_factor_bps']))
        for name, listing in spoke['assets'].items():
            expected = {field: listing[field] for field in ('liquidation_threshold', 'liquidation_bonus', 'liquidation_fees', 'supply_cap', 'borrow_cap')}
            expected.update(is_collateralizable=listing['can_be_collateral'], is_borrowable=listing['can_be_borrowed'], loan_to_value=listing['ltv'])
            expected.update({flag: listing.get(flag, False) for flag in ('paused', 'frozen', 'no_seize')})
            add('CONTROLLER', 'get_spoke_asset', dict(spoke_id=spoke_id,
                hub_asset=dict(hub_id=listing['hub_id'], asset=by_name[name]['asset_address'])), expected)
    return checks


def decimal_cases(root, networks):
    markets = load(root, 'markets.json')['markets']
    spokes = load(root, 'spokes.json')
    cases = []
    for decimals in (7, 8, 9, 18):
        amount = 10**decimals + 1
        candidates = [(market, config_id) for market in markets for config_id, spoke in spokes.items()
            if market['oracle']['asset_decimals'] == decimals
            and (listing := spoke['assets'].get(market['name']))
            and listing['can_be_collateral'] and int(listing['supply_cap']) >= amount]
        assert candidates, f'no collateral fixture for {decimals} decimals'
        market, config_id = candidates[0]
        cases.append(dict(name=market['name'], asset=market['asset_address'], decimals=decimals,
            hub_id=market['hub_id'], spoke_id=networks['testnet']['spoke_ids'][config_id],
            amount=str(amount), partial=str(10**decimals // 3 + 1)))
    return cases


def lending_case(root, networks):
    markets = {m['name']: m for m in load(root, 'markets.json')['markets']}
    debt = markets['USDC']
    for config_id, spoke in load(root, 'spokes.json').items():
        if not spoke['assets'].get('USDC', {}).get('can_be_borrowed'):
            continue
        for name, listing in spoke['assets'].items():
            market = markets[name]
            if listing['can_be_collateral'] and any('Xoxno' in node for node in walk(market['oracle'])):
                price = fixture_price(root, {'Token': market['asset_address']})
                amount = (200*10**18*10**market['oracle']['asset_decimals'] + price-1)//price
                assert amount <= int(listing['supply_cap'])
                return dict(asset=market['asset_address'], hub_id=listing['hub_id'], amount=str(amount),
                    spoke_id=networks['testnet']['spoke_ids'][config_id], debt=debt['asset_address'], debt_hub=debt['hub_id'])
    raise AssertionError('no enabled XOXNO collateral with a borrowable USDC market')


def verify_price(root, name, observed):
    market = next(m for m in load(root, 'markets.json')['markets'] if m['name'] == name)
    assert len(observed) == 1, 'expected one resolved price'
    key, quote = next(iter(observed.items()))
    verify_equal({'Token': market['asset_address']}, json.loads(key))
    assert isinstance(quote['price_wad'], (str, int)) and not isinstance(quote['price_wad'], bool)
    assert re.fullmatch(r'[0-9]+', str(quote['price_wad'])), 'invalid price'
    price = int(quote['price_wad'])
    oracle = market['oracle']
    verify_equal(oracle['asset_decimals'], quote['asset_decimals'])
    assert int(oracle['min_sanity_price_wad']) <= price <= int(oracle['max_sanity_price_wad']), 'price outside production sanity band'
    expected = fixture_price(root, {'Token': market['asset_address']})
    assert price == expected, f'fixture reference price {price} != {expected}'


def fixture_price(root, key):
    """Independent integer valuation from seeded providers and LP reserves.

    Mirrors specified rounding, never reads the contract's calculated price.
    This checks fixture-backed composition; external providers are separate.
    """
    config, fixtures = load(root, 'markets.json'), load(root, 'fixtures.json')
    entries = {json.dumps(e.get('key', {'Token': e.get('asset_address')}), sort_keys=True): e['oracle']
               for e in config.get('references', []) + config['markets']}
    def half_up(n, d):
        return (2 * n + d) // (2 * d)
    def feed(spec):
        kind, provider = next(iter(spec['provider'].items()))
        identity = provider.get('feed_id', provider.get('asset'))
        if isinstance(identity, dict) and 'Symbol' in identity:
            identity = {'Other': identity['Symbol']}
        seeds = [seed for seed in fixtures['seeds'] if seed['kind'] == kind
                 and seed['contract'] == provider['contract']
                 and seed.get('feed', seed.get('asset')) == identity]
        assert seeds, 'missing provider seed'
        unit = 10**4 if kind == 'Reflector' else 10**10
        return int(seeds[-1]['price']) // unit * unit
    def resolve(price_key, stack=()):
        k = json.dumps(price_key, sort_keys=True)
        assert k not in stack, 'cyclic fixture price'
        oracle = entries[k]
        values = []
        for source in oracle['sources']:
            kind, spec = next(iter(source.items()))
            if kind == 'Feed':
                value = feed(spec)
            elif kind == 'Scaled':
                value = half_up(feed(spec['factor']) * resolve(spec['quote'], stack + (k,)), 10**18)
            else:
                assert kind in ('AquariusLp', 'AquariusStableLp')
                pool = next(p['snapshot'] for p in fixtures['pools'] if p['contract'] == spec['pool'])
                a, b = map(int, pool['reserves'])
                pa, pb = (resolve(spec['key_' + leg], stack + (k,)) for leg in ('a', 'b'))
                da, db = spec['reserve_a_decimals'], spec['reserve_b_decimals']
                shares = int(pool['shares']) * 10**(18 - oracle['asset_decimals'])
                assert min(a, b, pa, pb, shares) > 0
                if kind == 'AquariusLp':
                    va, vb = half_up(a * pa, 10**da), half_up(b * pb, 10**db)
                    value = 2 * isqrt(va * vb) * 10**18 // shares
                else:
                    x, y = a * 10**(18-da), b * 10**(18-db)
                    ann = 2 * int(pool['amplification'])
                    d = x + y
                    for _ in range(255):
                        product = ((d * d // (2*x)) * d) // (2*y)
                        next_d = (ann*(x+y) + 2*product)*d // ((ann-1)*d + 3*product)
                        if abs(d-next_d) <= 1:
                            value = next_d * min(pa, pb) // shares
                            break
                        d = next_d
                    else:
                        raise AssertionError('stable invariant did not converge')
            values.append(value)
        assert 1 <= len(values) <= 2
        return sum(values) // len(values)
    return resolve(key)


def plan(root):
    markets = load(root, 'markets.json')
    definitions = {}
    enabled = [m for m in markets['markets'] if m.get('enabled', True)]
    for m in enabled:
        definitions[m['asset_address']] = dict(kind='token', decimals=m['oracle']['asset_decimals'], name=m['name'])
    for node in walk([enabled, markets.get('references', [])]):
        for kind in ('Reflector', 'RedStone', 'Xoxno'):
            if kind in node:
                definitions[node[kind]['contract']] = dict(kind=kind)
        for kind in ('AquariusLp', 'AquariusStableLp'):
            if kind in node:
                definitions[node[kind]['pool']] = dict(kind='pool', decimals=7, name='LPPOOL')
    return definitions


def materialize(source, output, mapping, network):
    output.mkdir(parents=True, exist_ok=True)
    assert set(mapping) == set(plan(source)), 'missing or extraneous fixture addresses'
    assert len(set(mapping.values())) == len(mapping), 'provider/token fixture aliasing'
    def remap(value):
        if isinstance(value, str):
            return mapping.get(value, value)
        if isinstance(value, list):
            return [remap(v) for v in value]
        if isinstance(value, dict):
            return {k: remap(v) for k, v in value.items()}
        return value
    original = load(source, 'markets.json')
    original['markets'] = [m for m in original['markets'] if m.get('enabled', True)]
    prices = {}
    for ref in original.get('references', []):
        prices[json.dumps(ref['key'], sort_keys=True)] = (int(ref['oracle']['min_sanity_price_wad']) + int(ref['oracle']['max_sanity_price_wad'])) // 2
    for m in original['markets']:
        oracle = m['oracle']
        price = (int(oracle['min_sanity_price_wad']) + int(oracle['max_sanity_price_wad'])) // 2
        if m['name'] == 'PYUSD':
            # Distinct stable-leg prices exercise conservative min-price valuation.
            price = (int(oracle['min_sanity_price_wad']) + 3*int(oracle['max_sanity_price_wad'])) // 4
        for s in oracle['sources']:
            if 'Scaled' in s:
                scaled = s['Scaled']
                price = prices[json.dumps(scaled['quote'], sort_keys=True)] * ((int(scaled['min_factor_wad']) + int(scaled['max_factor_wad'])) // 2) // 10**18
        prices[json.dumps({'Token':m['asset_address']}, sort_keys=True)] = price
    seeds, pools, bases = [], [], {}
    def feed(feed_, price):
        provider = feed_['provider']
        if 'Reflector' in provider:
            p = provider['Reflector']
            asset = p['asset']
            asset = {'Other':asset['Symbol']} if 'Symbol' in asset else asset
            seeds.append(dict(kind='Reflector', contract=mapping[p['contract']], asset=remap(asset), price=str(price)))
        elif 'RedStone' in provider or 'Xoxno' in provider:
            kind='RedStone' if 'RedStone' in provider else 'Xoxno'
            p = provider[kind]
            seeds.append(dict(kind=kind, contract=mapping[p['contract']], feed=p['feed_id'], price=str(price)))
        else:
            raise ValueError(f'provider fixture unsupported: {provider}')
    for entry in original.get('references', []) + original['markets']:
        key = entry.get('key', {'Token':entry.get('asset_address')})
        price = prices[json.dumps(key,sort_keys=True)]
        for source_ in entry['oracle']['sources']:
            if 'Feed' in source_:
                feed(source_['Feed'],price)
            elif 'Scaled' in source_:
                s=source_['Scaled']; quote=prices[json.dumps(s['quote'],sort_keys=True)]
                feed(s['factor'], price*10**18//quote)
                if 'Reflector' in s['factor']['provider']:
                    p=s['factor']['provider']['Reflector']['contract']
                    q=s['quote']; bases[mapping[p]]=remap({'Stellar':q['Token']} if 'Token' in q else {'Other':q['Ref']})
            else:
                kind=next(iter(source_)); s=source_[kind]
                pa=prices[json.dumps(s['key_a'],sort_keys=True)]; pb=prices[json.dumps(s['key_b'],sort_keys=True)]
                # Equal USD reserves; comfortably above production depth floors.
                usd=10**26
                pools.append(dict(contract=mapping[s['pool']],snapshot=dict(
                    tokens=[mapping[s['token_a']],mapping[s['token_b']]],
                    reserves=[str(usd*10**s['reserve_a_decimals']//pa),str(usd*10**s['reserve_b_decimals']//pb)],
                    shares=str(2*usd*10**entry['oracle']['asset_decimals']//price),
                    share_token=mapping[entry['asset_address']], stable=kind=='AquariusStableLp',amplification='100')))
                if kind == 'AquariusStableLp':
                    # An imbalanced pool makes amplification/invariant math observable.
                    snapshot = pools[-1]['snapshot']
                    snapshot['reserves'][1] = str(int(snapshot['reserves'][1])*3//2)
                    snapshot['shares'] = str(int(snapshot['shares'])*5//4)
    markets=remap(original); markets['network']=network
    (output/'markets.json').write_text(json.dumps(markets,indent=2)+'\n')
    for name in ('hubs.json','spokes.json'):
        config=load(source,name)
        if name=='spokes.json':
            config={k:v for k,v in config.items() if v.get('enabled',True)}
            for spoke in config.values():
                spoke['assets']={k:v for k,v in spoke['assets'].items() if v.get('enabled',True)}
        (output/name).write_text(json.dumps(remap(config),indent=2)+'\n')
    (output/'blend.json').write_text('{"pools":[]}\n')
    (output/'oracle_feeds.json').write_text('{"feeds":[]}\n')
    evidence=dict(source_sha256=hashlib.sha256((source/'markets.json').read_bytes()).hexdigest(),
                  source_files_sha256={name:hashlib.sha256((source/name).read_bytes()).hexdigest() for name in ('markets.json','hubs.json','spokes.json')},
                  address_mapping=mapping, provider_execution='fixture', seeds=seeds,pools=pools,bases=bases,
                  prices={json.dumps(remap(json.loads(key)),sort_keys=True):str(price) for key,price in prices.items()})
    (output/'fixtures.json').write_text(json.dumps(evidence,indent=2)+'\n')


if __name__=='__main__':
    mode,source=sys.argv[1],Path(sys.argv[2])
    if mode=='plan':
        print(json.dumps(plan(source)))
    elif mode=='materialize':
        materialize(source,Path(sys.argv[3]),json.loads(Path(sys.argv[4]).read_text()),'testnet')
    elif mode=='checks':
        print(json.dumps(policy_checks(source, json.loads(Path(sys.argv[3]).read_text()))))
    elif mode=='decimals':
        print(json.dumps(decimal_cases(source, json.loads(Path(sys.argv[3]).read_text()))))
    elif mode=='lending':
        print(json.dumps(lending_case(source, json.loads(Path(sys.argv[3]).read_text()))))
    elif mode=='verify':
        check=json.loads(source.read_text())[int(sys.argv[3])]
        actual=json.loads(Path(sys.argv[4]).read_text())
        verify_equal(check['expected'], actual[check['field']] if 'field' in check else actual)
    elif mode=='verify-price':
        verify_price(source, sys.argv[3], json.loads(Path(sys.argv[4]).read_text()))
    else:
        raise SystemExit('expected plan/materialize/checks/decimals/verify/verify-price')
