"""A tiny client for `acidtrip mcp --headless` (JSON-RPC over stdio)."""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SITE = os.path.dirname(HERE)


def default_bin():
    if len(sys.argv) > 1:
        return sys.argv[1]
    target = os.environ.get('CARGO_TARGET_DIR', os.path.join(SITE, '..', 'target'))
    return os.path.join(target, 'debug', 'acidtrip')


class Mcp:
    def __init__(self, bin=None):
        self.p = subprocess.Popen([bin or default_bin(), 'mcp', '--headless'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True)
        self.i = 0
        self.rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                                'clientInfo': {'name': 'headings', 'version': '1'}})
        self.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})

    def send(self, msg):
        self.p.stdin.write(json.dumps(msg) + '\n')
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.i += 1
        self.send({'jsonrpc': '2.0', 'id': self.i, 'method': method, 'params': params})
        while True:
            r = json.loads(self.p.stdout.readline())
            if r.get('id') == self.i:
                return r

    def tool(self, tool_name, **args):
        r = self.rpc('tools/call', {"name": tool_name, "arguments": args})
        return ' '.join(c.get('text', '') for c in r.get('result', {}).get('content', []) if c.get('type') == 'text')
