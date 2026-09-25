"""Offline failure injection for the multi-hub liquidation's exact predicates."""
import copy
import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / 'flows/liquidation.sh').read_text()
RAY, UNIT = 10**27, 10**20


def predicate(name, values):
    # Execute the actual inline predicate; copies would only test the fixture.
    source = SOURCE.split("<<'" + name + "'\n", 1)[1].split('\n' + name, 1)[0]
    return subprocess.run([sys.executable, '-c', source, *[
        json.dumps(value) if isinstance(value, (dict, list)) else str(value)
        for value in values
    ]], capture_output=True, text=True)


def positions(hub, debt):
    return [{json.dumps(dict(hub_id=hub, asset=asset)): dict(scaled_amount=str(amount * RAY))}
            for asset, amount in [('H', 1000), ('I', debt)]]


class MultiHubLiquidation(unittest.TestCase):
    def test_secondary_book_usage_and_positions_stay_exact(self):
        before = [positions(2, 100),
                  dict(state=dict(cash='10000000000', supply_index=str(RAY), borrowed='0')),
                  dict(supplied_scaled_ray=str(1000 * RAY), borrowed_scaled_ray='0'),
                  dict(state=dict(cash='19000000000', borrow_index=str(RAY), borrowed=str(100 * RAY))),
                  dict(supplied_scaled_ray=str(2000 * RAY), borrowed_scaled_ray=str(100 * RAY))]
        self.assertEqual(predicate('PYLMHISOLATION', [*before, *copy.deepcopy(before), 2, 'H', 'I']).returncode, 0)
        for index, field in [(1, 'cash'), (1, 'supply_index'), (3, 'borrowed'), (3, 'borrow_index'),
                             (2, 'supplied_scaled_ray'), (4, 'borrowed_scaled_ray')]:
            with self.subTest(index=index, field=field):
                after = copy.deepcopy(before)
                target = after[index].get('state', after[index])
                target[field] = str(int(target[field]) + 1)
                self.assertNotEqual(predicate('PYLMHISOLATION', [*before, *after, 2, 'H', 'I']).returncode, 0)
        for side in (0, 1):
            after = copy.deepcopy(before)
            next(iter(after[0][side].values()))['scaled_amount'] = '0'
            self.assertNotEqual(predicate('PYLMHISOLATION', [*before, *after, 2, 'H', 'I']).returncode, 0)
        # Identical snapshots are insufficient when accidentally reading hub 1.
        self.assertNotEqual(predicate('PYLMHISOLATION', [*before, *before, 1, 'H', 'I']).returncode, 0)
        self.assertNotEqual(predicate('PYLMHISOLATION', [*before, *before, 2, 'I', 'H']).returncode, 0)

    def test_primary_usage_matches_exact_burns(self):
        before = positions(1, 600)
        after = copy.deepcopy(before)
        gross = 1666571428
        next(iter(after[0].values()))['scaled_amount'] = str(1000 * RAY - gross * UNIT)
        next(iter(after[1].values()))['scaled_amount'] = str(500 * RAY)
        coll_pre = dict(supplied_scaled_ray=str(1000 * RAY), borrowed_scaled_ray='0')
        debt_pre = dict(supplied_scaled_ray=str(2000 * RAY), borrowed_scaled_ray=str(600 * RAY))
        coll_post = {**coll_pre, 'supplied_scaled_ray': str(1000 * RAY - gross * UNIT)}
        debt_post = {**debt_pre, 'borrowed_scaled_ray': str(500 * RAY)}
        values = [before, after, coll_pre, debt_pre, coll_post, debt_post, gross, 1, 'H', 'I']
        self.assertEqual(predicate('PYLMHUSAGE', values).returncode, 0)
        for index, field in [(4, 'supplied_scaled_ray'), (4, 'borrowed_scaled_ray'),
                             (5, 'supplied_scaled_ray'), (5, 'borrowed_scaled_ray')]:
            with self.subTest(index=index, field=field):
                changed = copy.deepcopy(values)
                changed[index][field] = str(int(changed[index][field]) + 1)
                self.assertNotEqual(predicate('PYLMHUSAGE', changed).returncode, 0)
        changed = copy.deepcopy(values)
        changed[6] += 1
        self.assertNotEqual(predicate('PYLMHUSAGE', changed).returncode, 0)
        changed = copy.deepcopy(values)
        changed[7] = 2
        self.assertNotEqual(predicate('PYLMHUSAGE', changed).returncode, 0)


if __name__ == '__main__':
    unittest.main()
