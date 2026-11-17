"""Private credential fallback, entirely synthetic and offline."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import external_analysis as analysis


class CredentialFallback(unittest.TestCase):
    def test_fallback_boundaries(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {'XDG_CONFIG_HOME': tmp}, clear=True):
            root = Path(tmp)
            path = root / 'musializer/credentials.json'
            path.parent.mkdir()
            path.write_text(json.dumps({'schema': 'musializer.assist-credentials/v1',
                'entries': {'openrouter/default': {'secret': 'synthetic-file'}}}))
            path.chmod(0o600)
            (root / '.env').write_text('OPENROUTER_API_KEY=synthetic-repo\n')
            with patch.object(analysis, 'ROOT', root):
                self.assertEqual(analysis._openrouter_env()['OPENROUTER_API_KEY'], 'synthetic-file')
                self.assertNotIn('OPENROUTER_API_KEY', analysis._openrouter_env(allow_dotenv=False))
                with patch.dict(os.environ, {'OPENROUTER_API_KEY': 'synthetic-env'}):
                    path.chmod(0o644)
                    self.assertEqual(analysis._openrouter_env()['OPENROUTER_API_KEY'], 'synthetic-env')
                with self.assertRaisesRegex(ValueError, 'owned by this user.*0600'):
                    analysis._openrouter_env()
                path.chmod(0o600)
                with patch.object(analysis.os, 'getuid', return_value=os.getuid()+1):
                    with self.assertRaisesRegex(ValueError, 'owned by this user'):
                        analysis._openrouter_env()
                path.write_text('invalid synthetic JSON')
                with self.assertRaisesRegex(ValueError, 'invalid schema'):
                    analysis._openrouter_env()
                path.unlink()
                self.assertNotIn('OPENROUTER_API_KEY', analysis._openrouter_env())

if __name__ == '__main__':
    unittest.main()
