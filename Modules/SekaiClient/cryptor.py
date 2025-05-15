import msgpack
from typing import List, Dict, Union
from cryptography.hazmat.backends import default_backend
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes


class SekaiCryptor:
    def __init__(self, key: Union[bytes, str], iv: Union[bytes, str]):
        self._aes_key = key
        self._aes_iv = iv

    @staticmethod
    async def padding(s):
        return s + (16 - len(s) % 16) * bytes([16 - len(s) % 16])

    async def pack(self, content: Union[Dict, List]) -> bytes:
        cipher = Cipher(algorithms.AES(self._aes_key), modes.CBC(self._aes_iv), backend=default_backend())
        encryptor = cipher.encryptor()
        ss = msgpack.packb(content, use_single_float=True)
        ss = await self.padding(ss)
        encrypted = encryptor.update(ss) + encryptor.finalize()
        return encrypted

    async def unpack(self, content: bytes) -> Dict:
        cipher = Cipher(algorithms.AES(self._aes_key), modes.CBC(self._aes_iv), backend=default_backend())
        decryptor = cipher.decryptor()
        decrypted = decryptor.update(content) + decryptor.finalize()
        return msgpack.unpackb(decrypted[: -decrypted[-1]], strict_map_key=False)
