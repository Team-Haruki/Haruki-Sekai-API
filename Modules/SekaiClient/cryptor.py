import msgpack
import asyncio
from typing import List, Dict, Union
from cryptography.hazmat.backends import default_backend
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes


class SekaiCryptor:
    def __init__(self, key: Union[bytes, str], iv: Union[bytes, str]):
        self._aes_key = key
        self._aes_iv = iv

    @staticmethod
    def _pad(s: bytes) -> bytes:
        pad_len = 16 - len(s) % 16
        return s + bytes([pad_len] * pad_len)

    @staticmethod
    def _unpad(s: bytes) -> bytes:
        return s[: -s[-1]]

    def _encrypt(self, content: Union[Dict, List]) -> bytes:
        cipher = Cipher(algorithms.AES(self._aes_key), modes.CBC(self._aes_iv), backend=default_backend())
        encryptor = cipher.encryptor()
        packed = msgpack.packb(content, use_single_float=True)
        padded = self._pad(packed)
        return encryptor.update(padded) + encryptor.finalize()

    def _decrypt(self, content: bytes) -> Dict:
        cipher = Cipher(algorithms.AES(self._aes_key), modes.CBC(self._aes_iv), backend=default_backend())
        decryptor = cipher.decryptor()
        decrypted = decryptor.update(content) + decryptor.finalize()
        unpadded = self._unpad(decrypted)
        return msgpack.unpackb(unpadded, strict_map_key=False)

    async def pack(self, content: Union[Dict, List]) -> bytes:
        return await asyncio.to_thread(self._encrypt, content)

    async def unpack(self, content: bytes) -> Dict:
        return await asyncio.to_thread(self._decrypt, content)
