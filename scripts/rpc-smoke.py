#!/usr/bin/env python3
"""Exercise the real development bridge in a disposable store, without a browser.

Build tillbh-devserver and the frontend first, then pass --server and --static.
This verifies HTTP/backend behavior; it does not qualify the desktop installer.
"""

import argparse
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", required=True, type=Path)
    parser.add_argument("--static", required=True, type=Path)
    args = parser.parse_args()
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    token = None

    def rpc(command, params=None, expect_ok=True):
        payload = json.dumps({"cmd": command, "token": token, "args": params or {}}).encode()
        req = urllib.request.Request(base + "/rpc", data=payload, headers={"Content-Type": "application/json"})
        with http.open(req, timeout=15) as response:
            result = json.load(response)
        assert result["ok"] == expect_ok, f"{command}: unexpected success/error response"
        return result["data"] if expect_ok else result["error"]

    with tempfile.TemporaryDirectory(prefix="tillbh-rpc-smoke-") as root:
        data = Path(root) / "data"
        log_path = Path(root) / "server.log"
        process = None

        def stop():
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)

        def start():
            nonlocal process
            with log_path.open("ab") as log:
                process = subprocess.Popen(
                    [str(args.server.resolve()), "--data-dir", str(data), "--port", str(port), "--static", str(args.static.resolve())],
                    stdout=log, stderr=subprocess.STDOUT, env={**os.environ, "TILLBH_IMAGE_SEARCH": "off"},
                )
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"bridge exited with {process.returncode}: {log_path.read_text()}")
                try:
                    rpc("app.ping")
                    return
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            raise TimeoutError("development bridge did not become ready")

        try:
            start()
            with http.open(base, timeout=15) as response:
                assert b"TILLBH" in response.read(), "built frontend was not served"
            assert not rpc("setup.status")["setup_complete"]
            rpc("setup.initialize", {
                "business_name": "RPC Smoke Store", "branch_name": "Main", "vat_rate_bp": 1000,
                "owner_name": "Smoke Owner", "owner_pin": "7391", "device_name": "Smoke Till", "device_code": "S01",
            })
            owner = rpc("auth.users")[0]["user_id"]
            rpc("auth.login", {"user_id": owner, "pin": "0000"}, expect_ok=False)
            token = rpc("auth.login", {"user_id": owner, "pin": "7391"})["token"]
            tax = rpc("tax.list")[0]["tax_rule_id"]
            rpc("products.create", {
                "name": "Smoke Juice", "tax_rule_id": tax, "price_minor": 1250, "cost_minor": 750,
                "barcodes": ["SMOKE001"], "opening_stock_milli": 10000,
            })
            shift = rpc("shift.open", {"opening_float_minor": 20000, "operation_id": str(uuid.uuid4())})
            cart = rpc("pos.scan", {"barcode": "SMOKE001"})["cart"]
            assert (cart["totals"]["total_minor"], cart["totals"]["tax_minor"]) == (1250, 114)
            request = {
                "cart_id": cart["cart_id"], "operation_id": str(uuid.uuid4()), "expected_total_minor": 1250,
                "tenders": [{"method": "cash", "amount_minor": 2000}],
            }
            sale = rpc("pos.finalize", request)
            assert (sale["total_minor"], sale["change_minor"]) == (1250, 750)
            replay = rpc("pos.finalize", request)
            assert replay["replayed"] and replay["sale_id"] == sale["sale_id"]
            receipt = rpc("receipts.preview", {"kind": "sale", "ref_id": sale["sale_id"]})
            assert "Smoke Juice" in receipt["text"] and "BHD 1.250" in receipt["text"]
            closed = rpc("shift.close", {
                "shift_id": shift["shift_id"], "counted_cash_minor": 21250, "operation_id": str(uuid.uuid4()),
            })["summary"]
            assert closed["status"] == "closed" and closed["variance_minor"] == 0
            stop()
            token = None
            start()
            assert rpc("setup.status")["setup_complete"]
            token = rpc("auth.login", {"user_id": owner, "pin": "7391"})["token"]
            assert rpc("shift.current") is None
            assert rpc("receipts.preview", {"kind": "sale", "ref_id": sale["sale_id"]})["text"] == receipt["text"]
            stop()
            with sqlite3.connect(f"file:{data / 'tillbh.db'}?mode=ro", uri=True) as db:
                assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
                assert db.execute("SELECT COUNT(*) FROM sales").fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM payments").fetchone()[0] == 1
                assert db.execute("SELECT SUM(qty_milli) FROM stock_levels").fetchone()[0] == 9000
            print("RPC SMOKE PASSED: served UI, clean setup, PIN rejection/login, product, shift, cash/VAT totals, replay, receipt, close, relaunch, persistence, stock, integrity")
        finally:
            stop()


if __name__ == "__main__":
    main()
