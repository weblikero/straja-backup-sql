"""Loopback-only FTP fixture. Never exposes files outside the provided test directory."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent/'test-output/transfer-test-deps'))
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer

root = Path(sys.argv[1]).resolve(strict=True)
port_file = Path(sys.argv[2])
auth = DummyAuthorizer()
auth.add_user('fixture', 'fixture-secret', str(root), perm='elradfmwMT')
handler = FTPHandler
handler.authorizer = auth
server = FTPServer(('127.0.0.1', 0), handler)
port_file.write_text(str(server.socket.getsockname()[1]))
server.serve_forever()
