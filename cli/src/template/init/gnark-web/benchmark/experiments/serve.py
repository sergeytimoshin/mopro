#!/usr/bin/env python3
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from functools import partial
import argparse
p=argparse.ArgumentParser();p.add_argument('root');p.add_argument('--port',type=int,default=3217);a=p.parse_args()
class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy','same-origin')
        self.send_header('Cross-Origin-Embedder-Policy','require-corp')
        self.send_header('Cache-Control','no-store')
        super().end_headers()
    def log_message(self,*_):pass
ThreadingHTTPServer(('127.0.0.1',a.port),partial(Handler,directory=a.root)).serve_forever()
