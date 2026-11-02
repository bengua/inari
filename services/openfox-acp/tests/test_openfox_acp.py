"""Tests unitaires du pont openfox-acp : fonctions pures du traducteur
JSON-RPC ↔ HTTP/WS et cadrage WebSocket (RFC 6455), sans réseau."""

import base64
import hashlib
import os
import socket
import struct
import threading
import unittest

import openfox_acp as oa


class TestToolKind(unittest.TestCase):
    def test_connus(self):
        self.assertEqual(oa.tool_kind("read_file"), "read")
        self.assertEqual(oa.tool_kind("Grep"), "search")
        self.assertEqual(oa.tool_kind("write"), "edit")
        self.assertEqual(oa.tool_kind("bash"), "execute")
        self.assertEqual(oa.tool_kind("web_fetch"), "fetch")
        self.assertEqual(oa.tool_kind("delete"), "delete")
        self.assertEqual(oa.tool_kind("think"), "think")

    def test_inconnu(self):
        self.assertEqual(oa.tool_kind("nimporte_quoi"), "other")


class TestToolTitle(unittest.TestCase):
    def test_commande(self):
        self.assertEqual(oa.tool_title("bash", {"command": "ls -la"}), "bash: ls -la")

    def test_chemin(self):
        self.assertEqual(oa.tool_title("read", {"path": "/etc/hosts"}), "read: /etc/hosts")

    def test_tronque_a_120(self):
        titre = oa.tool_title("bash", {"command": "x" * 200})
        self.assertEqual(len(titre), len("bash: ") + 120)

    def test_sans_argument(self):
        self.assertEqual(oa.tool_title("think", {}), "think")
        self.assertEqual(oa.tool_title("think", None), "think")


class TestResultText(unittest.TestCase):
    def test_chaine(self):
        self.assertEqual(oa.result_text("sortie"), "sortie")

    def test_dict_cles_usuelles(self):
        self.assertEqual(oa.result_text({"output": "ok"}), "ok")
        self.assertEqual(oa.result_text({"error": "boom"}), "boom")

    def test_dict_json_replie(self):
        self.assertEqual(oa.result_text({"code": 0}), '{"code": 0}')

    def test_tronque_a_4000(self):
        self.assertLessEqual(len(oa.result_text({"x": "y" * 9000})), 4000)


class TestPromptText(unittest.TestCase):
    def test_texte(self):
        blocs = [{"type": "text", "text": "bonjour"}]
        self.assertEqual(oa.prompt_text(blocs), "bonjour")

    def test_ressource(self):
        blocs = [{"type": "resource",
                  "resource": {"uri": "file:///a.py", "text": "print(1)"}}]
        self.assertEqual(oa.prompt_text(blocs),
                         '<file uri="file:///a.py">\nprint(1)\n</file>')

    def test_lien(self):
        blocs = [{"type": "resource_link", "uri": "file:///b.py"}]
        self.assertEqual(oa.prompt_text(blocs), "(file: file:///b.py)")

    def test_vide_et_none(self):
        self.assertEqual(oa.prompt_text([]), "")
        self.assertEqual(oa.prompt_text(None), "")


class TestModelCatalog(unittest.TestCase):
    PROVIDERS = {
        "activeProviderId": "local",
        "providers": [
            {"id": "local", "name": "Local", "models": [{"id": "qwen3-27b"}]},
            {"id": "api", "name": "API", "models": [{"id": "k3"}]},
        ],
    }

    def test_catalogue_et_courant_actif(self):
        models, current = oa.model_catalog(self.PROVIDERS)
        self.assertEqual([m["modelId"] for m in models], ["local/qwen3-27b", "api/k3"])
        self.assertEqual(current, "local/qwen3-27b")

    def test_modele_de_session_connu(self):
        _, current = oa.model_catalog(self.PROVIDERS, current="api/k3")
        self.assertEqual(current, "api/k3")

    def test_modele_de_session_inconnu(self):
        _, current = oa.model_catalog(self.PROVIDERS, current="autre/x")
        self.assertEqual(current, "local/qwen3-27b")

    def test_vide(self):
        models, current = oa.model_catalog({"providers": []})
        self.assertEqual(models, [])
        self.assertIsNone(current)


class TestConfig(unittest.TestCase):
    def test_config_dir_env(self):
        ancien = os.environ.get("OPENFOX_ACP_DIR")
        os.environ["OPENFOX_ACP_DIR"] = "/tmp/ofac-test"
        try:
            self.assertEqual(str(oa.config_dir()), "/tmp/ofac-test")
        finally:
            if ancien is None:
                del os.environ["OPENFOX_ACP_DIR"]
            else:
                os.environ["OPENFOX_ACP_DIR"] = ancien

    def test_url_par_defaut(self):
        import tempfile
        ancien_url = os.environ.pop("OPENFOX_URL", None)
        ancien_dir = os.environ.get("OPENFOX_ACP_DIR")
        with tempfile.TemporaryDirectory() as vide:
            os.environ["OPENFOX_ACP_DIR"] = vide  # aucun fichier « url »
            try:
                self.assertEqual(oa.openfox_url(), oa.DEFAULT_URL)
            finally:
                if ancien_dir is None:
                    del os.environ["OPENFOX_ACP_DIR"]
                else:
                    os.environ["OPENFOX_ACP_DIR"] = ancien_dir
                if ancien_url is not None:
                    os.environ["OPENFOX_URL"] = ancien_url


def _lire_trame_client(conn):
    """Lit une trame (masquée) envoyée par le client."""
    b1, b2 = struct.unpack("!BB", conn.recv(2))
    n = b2 & 0x7F
    if n == 126:
        n = struct.unpack("!H", conn.recv(2))[0]
    mask = conn.recv(4) if b2 & 0x80 else None
    payload = b""
    while len(payload) < n:
        morceau = conn.recv(n - len(payload))
        if not morceau:
            break
        payload += morceau
    if mask:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return b1 & 0x0F, payload


def _trame_serveur(opcode, payload, fin=True):
    b1 = (0x80 if fin else 0) | opcode
    n = len(payload)
    if n < 126:
        return struct.pack("!BB", b1, n) + payload
    return struct.pack("!BBH", b1, 126, n) + payload


def _serveur_ws(srv, recu):
    """Mini-serveur WebSocket : handshake, texte fragmenté + ping, lit le
    pong et un texte du client, ferme proprement."""
    conn, _ = srv.accept()
    conn.settimeout(10)
    try:
        head = b""
        while b"\r\n\r\n" not in head:
            head += conn.recv(4096)
        key = next(l.split(":", 1)[1].strip()
                   for l in head.decode("latin-1").split("\r\n")
                   if l.lower().startswith("sec-websocket-key"))
        accept = base64.b64encode(hashlib.sha1((key + oa.WS_GUID).encode()).digest()).decode()
        conn.sendall(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                      "Connection: Upgrade\r\n"
                      f"Sec-WebSocket-Accept: {accept}\r\n\r\n").encode())

        conn.sendall(_trame_serveur(0x1, b"bon", fin=False))  # fragment 1
        conn.sendall(_trame_serveur(0x0, b"jour"))            # fragment 2 (fin)
        conn.sendall(_trame_serveur(0x9, b"coucou"))          # ping → pong attendu

        for _ in range(2):  # texte du client et pong, dans un ordre quelconque
            opcode, payload = _lire_trame_client(conn)
            if opcode == 0xA:
                recu["pong"] = payload
            elif opcode == 0x1:
                recu["texte"] = payload.decode()

        conn.sendall(_trame_serveur(0x8, b""))  # fermeture propre
    finally:
        conn.close()
        srv.close()


class TestWebSocket(unittest.TestCase):
    def test_aller_retour(self):
        srv = socket.socket()
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        port = srv.getsockname()[1]
        recu = {}
        fil = threading.Thread(target=_serveur_ws, args=(srv, recu), daemon=True)
        fil.start()

        ws = oa.WebSocket(f"ws://127.0.0.1:{port}/flux")
        self.assertEqual(ws.recv(), "bonjour")  # texte fragmenté réassemblé
        ws.send_text("réponse du client")
        self.assertIsNone(ws.recv())            # traite le ping (pong), puis la fermeture
        ws.close()
        fil.join(10)
        self.assertEqual(recu.get("pong"), b"coucou", "le client doit répondre pong au ping")
        self.assertEqual(recu.get("texte"), "réponse du client")

    def test_handshake_refuse(self):
        srv = socket.socket()
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        port = srv.getsockname()[1]

        def refuse():
            conn, _ = srv.accept()
            conn.settimeout(10)
            try:
                head = b""
                while b"\r\n\r\n" not in head:
                    head += conn.recv(4096)
                conn.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            finally:
                conn.close()
                srv.close()

        fil = threading.Thread(target=refuse, daemon=True)
        fil.start()
        with self.assertRaises(oa.OpenFoxError):
            oa.WebSocket(f"ws://127.0.0.1:{port}/flux")
        fil.join(10)


if __name__ == "__main__":
    unittest.main()
