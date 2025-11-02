package orderedmsgpack

import (
	"bytes"
	"fmt"
	"io"
	"strconv"
	"strings"

	"github.com/iancoleman/orderedmap"
	"github.com/vmihailenco/msgpack/v5"
)

type JSONNum struct {
	Raw string
}

func (n JSONNum) MarshalJSON() ([]byte, error) {
	return []byte(n.Raw), nil
}

func makeJSONFloat(f float64, bits int) JSONNum {
	prec := 17

	raw := strconv.FormatFloat(f, 'f', prec, bits)

	if strings.Contains(raw, ".") {
		trimmed := strings.TrimRight(raw, "0")
		if strings.HasSuffix(trimmed, ".") {
			trimmed += "0"
		}
		raw = trimmed
	} else {
		raw += ".0"
	}
	return JSONNum{Raw: raw}
}

func MsgpackToOrderedMap(b []byte) (*orderedmap.OrderedMap, error) {
	return MsgpackToOrderedMapFromReader(bytes.NewReader(b))
}

func MsgpackToOrderedMapFromReader(r io.Reader) (*orderedmap.OrderedMap, error) {
	dec := msgpack.NewDecoder(r)
	dAny, err := decodeAnyOrdered(dec)
	if err != nil {
		return nil, err
	}
	if om, ok := dAny.(*orderedmap.OrderedMap); ok {
		return om, nil
	}
	return nil, fmt.Errorf("top-level value is %T, expected map/object", dAny)
}

// decodeNilOrBool decodes nil or boolean values
func decodeNilOrBool(dec *msgpack.Decoder, code byte) (any, bool, error) {
	if code == 0xc0 {
		if err := dec.DecodeNil(); err != nil {
			return nil, false, err
		}
		return nil, true, nil
	}
	if code == 0xc2 || code == 0xc3 {
		v, err := dec.DecodeBool()
		return v, true, err
	}
	return nil, false, nil
}

// decodeFloat decodes float32 or float64 values
func decodeFloat(dec *msgpack.Decoder, code byte) (any, bool, error) {
	if code == 0xca {
		f32, err := dec.DecodeFloat32()
		if err != nil {
			return nil, false, err
		}
		return makeJSONFloat(float64(f32), 32), true, nil
	}
	if code == 0xcb {
		f64, err := dec.DecodeFloat64()
		if err != nil {
			return nil, false, err
		}
		return makeJSONFloat(f64, 64), true, nil
	}
	return nil, false, nil
}

// isIntCode checks if code represents an integer type
func isIntCode(c byte) bool {
	return c <= 0x7f || c >= 0xe0 || (c >= 0xcc && c <= 0xcf) || (c >= 0xd0 && c <= 0xd3)
}

// isStringCode checks if code represents a string type
func isStringCode(c byte) bool {
	return (c >= 0xa0 && c <= 0xbf) || c == 0xd9 || c == 0xda || c == 0xdb
}

// isBinaryCode checks if code represents a binary type
func isBinaryCode(c byte) bool {
	return c == 0xc4 || c == 0xc5 || c == 0xc6
}

// isArrayCode checks if code represents an array type
func isArrayCode(c byte) bool {
	return (c >= 0x90 && c <= 0x9f) || c == 0xdc || c == 0xdd
}

// isMapCode checks if code represents a map type
func isMapCode(c byte) bool {
	return (c >= 0x80 && c <= 0x8f) || c == 0xde || c == 0xdf
}

// decodeArray decodes an array recursively
func decodeArray(dec *msgpack.Decoder) ([]any, error) {
	n, err := dec.DecodeArrayLen()
	if err != nil {
		return nil, err
	}
	out := make([]any, n)
	for i := 0; i < n; i++ {
		v, err := decodeAnyOrdered(dec)
		if err != nil {
			return nil, err
		}
		out[i] = v
	}
	return out, nil
}

// decodeMap decodes a map recursively into an ordered map
func decodeMap(dec *msgpack.Decoder) (*orderedmap.OrderedMap, error) {
	n, err := dec.DecodeMapLen()
	if err != nil {
		return nil, err
	}
	om := orderedmap.New()
	om.SetEscapeHTML(false)
	for i := 0; i < n; i++ {
		k, err := decodeAnyOrdered(dec)
		if err != nil {
			return nil, err
		}
		v, err := decodeAnyOrdered(dec)
		if err != nil {
			return nil, err
		}
		var key string
		if jn, ok := k.(JSONNum); ok {
			key = jn.Raw
		} else {
			key = fmt.Sprint(k)
		}
		om.Set(key, v)
	}
	return om, nil
}

func decodeAnyOrdered(dec *msgpack.Decoder) (any, error) {
	code, err := dec.PeekCode()
	if err != nil {
		return nil, err
	}

	// Try nil or bool first
	if v, handled, err := decodeNilOrBool(dec, code); handled {
		return v, err
	}

	// Try float
	if v, handled, err := decodeFloat(dec, code); handled {
		return v, err
	}

	// int / uint
	if isIntCode(code) {
		var v any
		if err := dec.Decode(&v); err != nil {
			return nil, err
		}
		return v, nil
	}

	// string
	if isStringCode(code) {
		return dec.DecodeString()
	}

	// binary
	if isBinaryCode(code) {
		return dec.DecodeBytes()
	}

	// array
	if isArrayCode(code) {
		return decodeArray(dec)
	}

	// map
	if isMapCode(code) {
		return decodeMap(dec)
	}

	// default: try generic decode
	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, err
	}
	return v, nil
}
