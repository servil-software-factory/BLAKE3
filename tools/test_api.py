#!/usr/bin/env python3
"""Controls for the contracts and declarations checked against rustdoc."""
import unittest

from api import check_traits, check_walk


class Contracts(unittest.TestCase):
    docs = '# Trait implementations\n\n- `Clone` copies the bytes.\n'
    walk = '// WALKED\n// Thing::new Thing::Clone\n// NOT_WALKED\n// Thing::Drop: A live token must be consumed.\n// END\n'
    ops = {'Thing::new', 'Thing::Clone', 'Thing::Drop'}

    def test_exact_inventory_passes(self):
        check_traits('Thing', {'Clone'}, self.docs)
        check_walk('walk', self.ops, self.walk)

    def test_new_trait_needs_a_contract_in_its_section(self):
        with self.assertRaisesRegex(ValueError, 'missing.*Debug'):
            check_traits('Thing', {'Clone', 'Debug'}, self.docs + '# Elsewhere\n- `Debug` prints it.\n')

    def test_a_contract_may_run_over_lines(self):
        check_traits('Thing', {'Clone', 'Debug'}, '# Trait implementations\n\n- `Clone` copies\n  the bytes.\n- `Debug` prints them.\n\nMore words.\n')
        with self.assertRaisesRegex(ValueError, 'malformed'):
            check_traits('Thing', {'Clone'}, '# Trait implementations\n\n- `Clone` copies the bytes\n')

    def test_a_type_with_no_trait_needs_no_section(self):
        check_traits('Thing', set(), 'A thing.\n')
        with self.assertRaisesRegex(ValueError, 'expected one'):
            check_traits('Thing', {'Clone'}, 'A thing.\n')

    def test_removed_trait_leaves_a_stale_contract(self):
        with self.assertRaisesRegex(ValueError, 'stale.*Clone'):
            check_traits('Thing', set(), self.docs)

    def test_new_operation_needs_a_walk_or_reason(self):
        with self.assertRaisesRegex(ValueError, 'missing.*Thing::reset'):
            check_walk('walk', self.ops | {'Thing::reset'}, self.walk)

    def test_removed_operation_leaves_a_stale_declaration(self):
        with self.assertRaisesRegex(ValueError, 'stale.*Thing::new'):
            check_walk('walk', self.ops - {'Thing::new'}, self.walk)

    def test_an_omission_needs_a_reason(self):
        with self.assertRaisesRegex(ValueError, 'reason'):
            check_walk('walk', self.ops, self.walk.replace(': A live token must be consumed.', ''))


if __name__ == '__main__':
    unittest.main()
