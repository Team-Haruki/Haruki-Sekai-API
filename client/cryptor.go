package client

import (
	"crypto/aes"
	"crypto/cipher"
	"encoding/hex"
	"errors"
	"fmt"
	"sync"

	"github.com/vgorin/cryptogo/pad"
	"github.com/vmihailenco/msgpack/v5"
)

type SekaiCryptor struct {
	key   []byte
	iv    []byte
	block cipher.Block
}

func NewSekaiCryptorFromHex(aesKeyHex, aesIVHex string) (*SekaiCryptor, error) {
	key, err := hex.DecodeString(aesKeyHex)
	if err != nil {
		return nil, fmt.Errorf("invalid aes key hex: %w", err)
	}
	iv, err := hex.DecodeString(aesIVHex)
	if err != nil {
		return nil, fmt.Errorf("invalid aes iv hex: %w", err)
	}
	if len(iv) != aes.BlockSize {
		return nil, fmt.Errorf("invalid iv length: got %d, want %d", len(iv), aes.BlockSize)
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, fmt.Errorf("new cipher: %w", err)
	}
	return &SekaiCryptor{
		key:   key,
		iv:    iv,
		block: block,
	}, nil
}

func (c *SekaiCryptor) newCBC(encrypt bool) cipher.BlockMode {
	if encrypt {
		return cipher.NewCBCEncrypter(c.block, c.iv)
	}
	return cipher.NewCBCDecrypter(c.block, c.iv)
}

func (c *SekaiCryptor) Pack(content any) ([]byte, error) {
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

	encrypter := c.newCBC(true)

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

func (c *SekaiCryptor) UnpackInto(content []byte, out any) error {
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
		return err
	}

	decrypter := c.newCBC(false)

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
		return fmt.Errorf("failed to unpad: %w", err)
	}

	if out == nil {
		return fmt.Errorf("out must be a non-nil pointer")
	}
	if err := msgpack.Unmarshal(unpadded, out); err != nil {
		return fmt.Errorf("failed to unmarshal: %w", err)
	}

	return nil
}

func (c *SekaiCryptor) Unpack(content []byte) (any, error) {
	var result any
	if err := c.UnpackInto(content, &result); err != nil {
		return nil, err
	}
	return result, nil
}

func UnpackInto[T any](c *SekaiCryptor, content []byte) (*T, error) {
	var v T
	if err := c.UnpackInto(content, &v); err != nil {
		return nil, err
	}
	return &v, nil
}
