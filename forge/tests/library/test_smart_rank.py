"""Library matching on planner wording, without the installed packs.

    engines/assetfurnace/.venv/bin/python -m unittest tests.library.test_smart_rank
"""

import unittest

from forge.library import smart
from forge.library.names import tokens


def row(name, pack="fantasy-props-megakit"):
    ident = "-".join(tokens(name))
    return {"id": f"{pack}/{ident}", "name": name, "plain_name": name, "pack": pack, "tags": tokens(name)}


ROWS = [row("Book Stand"), row("Anvil Log"), row("Candle Stick"), row("Lantern Wall"), row("Table Plate"),
        row("Table Spoon"), row("Table Large"), row("Bookcase 2"), row("Bed Twin1"), row("Crate Wooden"),
        row("Metal Plain 1", "downtown-city-megakit"), row("CommonTree_1", "stylized-nature-megakit"),
        row("Bush", "ultimate-stylized-nature")]


def best(ask):
    primary, description, _ = smart.phrases(ask)
    ranked = smart.rank(ROWS, primary, description)
    return (ranked[0][1]["name"], ranked[0][0]) if ranked else (None, 0)


class PlannerWording(unittest.TestCase):
    def test_a_differently_named_thing_is_not_a_match(self):
        # Asked by the item's head word, as Otto does first; the asset names some other object.
        for ask in ("stand. A black stand brings the microphone up to the cat's mouth.",
                    "log. A low log has rough bark and a smooth, flat top.",
                    "stick. A long stick has a thick end for a hand and a thin tip for a treat.",
                    "lantern. A little flame shines through the glass of an old lantern."):
            with self.subTest(ask=ask):
                self.assertLess(best(ask)[1], 0.65)

    def test_real_matches_still_match(self):
        for ask, name in (("Plate. A white plate has a gently raised rim.", "Table Plate"),
                          ("spoon. The wooden spoon has a long handle and a shallow bowl.", "Table Spoon"),
                          ("table. Four legs hold up a round wooden top.", "Table Large"),
                          ("crate. The crate has a flat wooden lid.", "Crate Wooden"),
                          ("bed. Four wooden legs lift the bed.", "Bed Twin1")):
            with self.subTest(ask=ask):
                found, score = best(ask)
                self.assertEqual(found, name)
                self.assertGreaterEqual(score, 0.65)

    def test_planner_words_for_pack_names(self):
        for ask, name in (("oak. A wide old oak stands behind the fire.", "CommonTree_1"),
                          ("hedge. A low green hedge runs along the garden.", "Bush"),
                          ("Bookshelf. Tall shelves full of books.", "Bookcase 2")):
            with self.subTest(ask=ask):
                found, score = best(ask)
                self.assertEqual(found, name)
                self.assertGreaterEqual(score, 0.65)

    def test_tableware_is_not_a_table_and_numbers_are_labels(self):
        self.assertEqual(best("table. A long table.")[0], "Table Large")
        self.assertEqual(best("2. A low log to sit on."), (None, 0))  # "Log seat 2": the 2 identifies nothing
        self.assertNotEqual(best("Log seat 1. A low log to sit on.")[0], "Metal Plain 1")


if __name__ == "__main__":
    unittest.main()
