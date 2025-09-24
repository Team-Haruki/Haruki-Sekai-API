package client

import (
	"crypto/aes"
	"crypto/cipher"
	"encoding/hex"
	"errors"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"

	"github.com/vgorin/cryptogo/pad"
	"github.com/vmihailenco/msgpack/v5"
)

func getCipher(server utils.SekaiRegion, encrypt bool) (cipher.BlockMode, error) {
	var key, iv []byte
	if server == utils.SekaiRegionEN {
		key, _ = hex.DecodeString(config.Cfg.SekaiClient.ENServerAESKey)
		iv, _ = hex.DecodeString(config.Cfg.SekaiClient.ENServerAESIV)
	} else {
		key, _ = hex.DecodeString(config.Cfg.SekaiClient.OtherServerAESKey)
		iv, _ = hex.DecodeString(config.Cfg.SekaiClient.OtherServerAESIV)
	}

	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}

	if encrypt {
		return cipher.NewCBCEncrypter(block, iv), nil
	}
	return cipher.NewCBCDecrypter(block, iv), nil
}

func Pack(content interface{}, server utils.SekaiRegion) ([]byte, error) {
	if content == nil {
		return nil, errors.New("content cannot be nil")
	}

	packed, err := msgpack.Marshal(content)
	if err != nil {
		return nil, err
	}

	if len(packed) == 0 {
		return nil, errors.New("packed content is empty")
	}

	padded := pad.PKCS7Pad(packed, aes.BlockSize)

	encrypter, err := getCipher(server, true)
	if err != nil {
		return nil, err
	}

	encrypted := make([]byte, len(padded))
	encrypter.CryptBlocks(encrypted, padded)

	return encrypted, nil
}

func Unpack(content []byte, server utils.SekaiRegion) (interface{}, error) {
	if len(content) == 0 {
		return nil, errors.New("content cannot be empty")
	}

	if len(content)%aes.BlockSize != 0 {
		return nil, errors.New("content length is not a multiple of AES block size")
	}

	decrypter, err := getCipher(server, false)
	if err != nil {
		return nil, err
	}

	decrypted := make([]byte, len(content))
	decrypter.CryptBlocks(decrypted, content)

	unpadded, err := pad.PKCS7Unpad(decrypted)
	if err != nil {
		return nil, err
	}

	var out interface{}
	if err := msgpack.Unmarshal(unpadded, &out); err != nil {
		return nil, err
	}

	return out, nil
}
