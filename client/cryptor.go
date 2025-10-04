package client

import (
	"crypto/aes"
	"crypto/cipher"
	"encoding/hex"
	"errors"
	"fmt"
	"haruki-sekai-api/config"
	"haruki-sekai-api/utils"
	"sync"

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

func Pack(content any, server utils.SekaiRegion) ([]byte, error) {
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

var (
	ErrEmptyContent     = errors.New("content cannot be empty")
	ErrInvalidBlockSize = errors.New("content length is not a multiple of AES block size")
	ErrDecryptionFailed = errors.New("failed to decrypt content")
)

var bytesPool = sync.Pool{
	New: func() any {
		b := make([]byte, 0, 1024)
		return &b
	},
}

func UnpackInto[T any](content []byte, server utils.SekaiRegion) (*T, error) {
	validateContent := func(content []byte) error {
		if len(content) == 0 {
			return ErrEmptyContent
		}
		if len(content)%aes.BlockSize != 0 {
			return ErrInvalidBlockSize
		}
		return nil
	}

	if err := validateContent(content); err != nil {
		return nil, err
	}

	decrypter, err := getCipher(server, false)
	if err != nil {
		return nil, fmt.Errorf("failed to create cipher: %w", err)
	}

	decrypted := bytesPool.Get().(*[]byte)
	if cap(*decrypted) < len(content) {
		*decrypted = make([]byte, len(content))
	} else {
		*decrypted = (*decrypted)[:len(content)]
	}
	defer bytesPool.Put(decrypted)

	decrypter.CryptBlocks(*decrypted, content)

	unpadded, err := pad.PKCS7Unpad(*decrypted)
	if err != nil {
		return nil, fmt.Errorf("failed to unpad: %w", err)
	}

	var result T
	if err := msgpack.Unmarshal(unpadded, &result); err != nil {
		return nil, fmt.Errorf("failed to unmarshal: %w", err)
	}

	return &result, nil
}

func Unpack(content []byte, server utils.SekaiRegion) (any, error) {
	return UnpackInto[any](content, server)
}
