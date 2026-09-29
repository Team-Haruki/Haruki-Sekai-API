"""Exercise dump parsing, UTF-8 I/O and failure before any model is written."""
import pathlib
import tempfile
import unittest
from unittest import mock

import generate_mysekai_followups as generator


def dump(fields='[Key("id")]\npublic int id;'):
    return "\n".join(
        f"public class Master{name} // TypeDefIndex: 1\n{{\n{body}\n}}"
        for name, body in [
            ("MysekaiTool", fields),
            ("MysekaiSite", fields),
            ("MysekaiCharacterTalkPreAction", fields),
            ("MysekaiBlueprintShop", '[Key("mysekaiBlueprintShopItemLotteryType")]\npublic string lottery;'),
            ("MysekaiMaterialPossession", fields),
        ]
    )


class GeneratorTests(unittest.TestCase):
    def test_preserves_field_names_types_and_optional_values(self):
        source = dump('''[Key("id")]
public int id;
[Key("coolTimeMicroSeconds")]
public float coolTimeMicroSeconds;
[Key("name")]
public string name;
[Key("isBase")]
public bool isBase;
// 日本語コメント
''')
        models = generator.generate_models(source)
        self.assertEqual(len(models), 5)
        blueprint = models['mysekaiblueprintshops.rs']
        self.assertIn('pub mysekai_blueprint_shop_item_lottery_type: Option<String>', blueprint)
        self.assertNotIn('pub id:', blueprint)
        tool = models['mysekaitools.rs']
        self.assertIn('pub type Mysekaitool = Vec<MysekaitoolElement>;', tool)
        self.assertIn('pub cool_time_micro_seconds: Option<f64>', tool)
        self.assertIn('pub name: Option<String>', tool)
        self.assertIn('pub is_base: Option<bool>', tool)
        self.assertIn('#[serde(rename_all = "camelCase")]', tool)

    def test_rejects_missing_class(self):
        with self.assertRaisesRegex(ValueError, 'Missing MasterMysekaiTool'):
            generator.generate_models('')

    def test_rejects_empty_or_unexpected_identity(self):
        for fields in ['', '[Key("id")]\npublic string id;']:
            with self.subTest(fields=fields), self.assertRaisesRegex(ValueError, 'Unexpected fields'):
                generator.generate_models(dump(fields))

    def test_rejects_unknown_type(self):
        with self.assertRaises(KeyError):
            generator.generate_models(dump('[Key("id")]\npublic int id;\n[Key("other")]\npublic Custom other;'))

    def test_utf8_io_and_failure_preserves_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'dump').mkdir()
            output = root / 'models'
            output.mkdir()
            source = root / 'dump/dump.cs'
            source.write_text(dump() + '\n// 日本語', encoding='utf-8')
            with mock.patch.object(generator, 'OUTPUT_DIR', output), mock.patch.object(pathlib.Path, 'cwd', return_value=root):
                generator.main()
                saved = {p.name: p.read_bytes() for p in output.iterdir()}
                self.assertEqual(len(saved), 5)
                generator.main()
                self.assertEqual(saved, {p.name: p.read_bytes() for p in output.iterdir()})
                source.write_text('invalid dump', encoding='utf-8')
                with self.assertRaises(ValueError):
                    generator.main()
                self.assertEqual(saved, {p.name: p.read_bytes() for p in output.iterdir()})


if __name__ == '__main__':
    unittest.main()
