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

func isIntCode(c byte) bool {
	return c <= 0x7f || c >= 0xe0 || (c >= 0xcc && c <= 0xcf) || (c >= 0xd0 && c <= 0xd3)
}

func isStringCode(c byte) bool {
	return (c >= 0xa0 && c <= 0xbf) || c == 0xd9 || c == 0xda || c == 0xdb
}

func isBinaryCode(c byte) bool {
	return c == 0xc4 || c == 0xc5 || c == 0xc6
}

func isArrayCode(c byte) bool {
	return (c >= 0x90 && c <= 0x9f) || c == 0xdc || c == 0xdd
}

func isMapCode(c byte) bool {
	return (c >= 0x80 && c <= 0x8f) || c == 0xde || c == 0xdf
}

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

	if v, handled, err := decodeNilOrBool(dec, code); handled {
		return v, err
	}
	if v, handled, err := decodeFloat(dec, code); handled {
		return v, err
	}
	if isIntCode(code) {
		var v any
		if err := dec.Decode(&v); err != nil {
			return nil, err
		}
		return v, nil
	}
	if isStringCode(code) {
		return dec.DecodeString()
	}
	if isBinaryCode(code) {
		return dec.DecodeBytes()
	}
	if isArrayCode(code) {
		return decodeArray(dec)
	}
	if isMapCode(code) {
		return decodeMap(dec)
	}

	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, err
	}
	return v, nil
}
