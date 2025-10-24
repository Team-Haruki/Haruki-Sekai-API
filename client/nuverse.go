package client

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strconv"

	"github.com/bytedance/sonic"
	"github.com/iancoleman/orderedmap"
)

func loadStructures(path string) (*orderedmap.OrderedMap, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	om := orderedmap.New()
	om.SetEscapeHTML(false)
	if err := sonic.Unmarshal(data, om); err != nil {
		return nil, err
	}
	return om, nil
}

func RestoreDict(arrayData []interface{}, keyStructure []interface{}) *orderedmap.OrderedMap {
	result := orderedmap.New()
	result.SetEscapeHTML(false)

	// 辅助函数：处理 __tuple__ 逻辑
	handleTuple := func(second interface{}, i int) *orderedmap.OrderedMap {
		var tupleKeys []interface{}
		switch s := second.(type) {
		case *orderedmap.OrderedMap:
			if tupleKeysRaw, found := s.Get("__tuple__"); found {
				tupleKeys, _ = tupleKeysRaw.([]interface{})
			}
		case orderedmap.OrderedMap:
			if tupleKeysRaw, found := s.Get("__tuple__"); found {
				tupleKeys, _ = tupleKeysRaw.([]interface{})
			}
		case map[string]interface{}:
			if tupleKeysRaw, found := s["__tuple__"]; found {
				tupleKeys, _ = tupleKeysRaw.([]interface{})
			}
		}

		if tupleKeys != nil {
			dict := orderedmap.New()
			dict.SetEscapeHTML(false)

			if i < len(arrayData) && arrayData[i] != nil {
				if tupleVals, ok := arrayData[i].([]interface{}); ok {
					for j, v := range tupleVals {
						if j >= len(tupleKeys) {
							break
						}
						if v != nil {
							if keyStr, ok := tupleKeys[j].(string); ok {
								dict.Set(keyStr, v)
							}
						}
					}
				}
			}
			return dict
		}
		return nil
	}

	if len(keyStructure) == 2 {
		if keyName, ok := keyStructure[0].(string); ok {
			var tupleKeys []interface{}
			switch second := keyStructure[1].(type) {
			case *orderedmap.OrderedMap:
				if tupleKeysRaw, found := second.Get("__tuple__"); found {
					tupleKeys, _ = tupleKeysRaw.([]interface{})
				}
			case orderedmap.OrderedMap:
				if tupleKeysRaw, found := second.Get("__tuple__"); found {
					tupleKeys, _ = tupleKeysRaw.([]interface{})
				}
			case map[string]interface{}:
				if tupleKeysRaw, found := second["__tuple__"]; found {
					tupleKeys, _ = tupleKeysRaw.([]interface{})
				}
			}

			if tupleKeys != nil {
				dict := orderedmap.New()
				dict.SetEscapeHTML(false)

				tupleVals := arrayData
				if len(arrayData) == 1 {
					if innerArr, ok := arrayData[0].([]interface{}); ok {
						tupleVals = innerArr
					}
				}

				for j, v := range tupleVals {
					if j >= len(tupleKeys) {
						break
					}
					if v != nil {
						if keyStr, ok := tupleKeys[j].(string); ok {
							dict.Set(keyStr, v)
						}
					}
				}

				result.Set(keyName, dict)
				return result
			}
		}
	}

	for i, key := range keyStructure {
		switch k := key.(type) {

		case []interface{}:
			if len(k) < 2 {
				continue
			}
			keyName, ok := k[0].(string)
			if !ok {
				continue
			}

			switch second := k[1].(type) {

			case *orderedmap.OrderedMap, orderedmap.OrderedMap, map[string]interface{}:
				if dict := handleTuple(second, i); dict != nil {
					result.Set(keyName, dict)
				}

			case []interface{}:
				subList := make([]*orderedmap.OrderedMap, 0)
				if i < len(arrayData) {
					if arr, ok := arrayData[i].([]interface{}); ok {
						for _, sub := range arr {
							if subArr, ok := sub.([]interface{}); ok {
								if len(second) > 0 {
									if innerStruct, ok := second[0].([]interface{}); ok && len(innerStruct) >= 2 {
										subList = append(subList, RestoreDict(subArr, innerStruct))
									} else {
										subList = append(subList, RestoreDict(subArr, second))
									}
								} else {
									subList = append(subList, RestoreDict(subArr, second))
								}
							} else {
								subList = append(subList, orderedmap.New())
							}
						}
					}
				}
				result.Set(keyName, subList)
			}

		case string:
			if i < len(arrayData) && arrayData[i] != nil {
				result.Set(k, arrayData[i])
			}
		}
	}
	return result
}

func RestoreCompactData(data *orderedmap.OrderedMap) []*orderedmap.OrderedMap {
	var (
		columnLabels []string
		columns      [][]interface{}
	)

	var enumOM *orderedmap.OrderedMap
	if v, ok := data.Get("__ENUM__"); ok {
		switch em := v.(type) {
		case *orderedmap.OrderedMap:
			enumOM = em
		case map[string]any:
			om := orderedmap.New()
			om.SetEscapeHTML(false)
			keys := make([]string, 0, len(em))
			for k := range em {
				keys = append(keys, k)
			}
			sort.Strings(keys)
			for _, k := range keys {
				om.Set(k, em[k])
			}
			enumOM = om
		}
	}

	for _, key := range data.Keys() {
		if key == "__ENUM__" {
			continue
		}
		columnLabels = append(columnLabels, key)

		var dataColumn []interface{}
		if val, ok := data.Get(key); ok {
			if vSlice, ok := val.([]interface{}); ok {
				dataColumn = vSlice
			} else {
				dataColumn = []interface{}{}
			}
		} else {
			dataColumn = []interface{}{}
		}

		if enumOM != nil {
			if enumColRaw, ok := enumOM.Get(key); ok {
				var enumSlice []interface{}
				switch e := enumColRaw.(type) {
				case []interface{}:
					enumSlice = e
				case *orderedmap.OrderedMap:
					keys := e.Keys()
					allNum := true
					idx := make([]int, len(keys))
					for i, k := range keys {
						n, err := strconv.Atoi(k)
						if err != nil {
							allNum = false
							break
						}
						idx[i] = n
					}
					if allNum {
						order := make([]int, len(keys))
						for i := range order {
							order[i] = i
						}
						sort.Slice(order, func(i, j int) bool { return idx[order[i]] < idx[order[j]] })
						max := -1
						for _, n := range idx {
							if n > max {
								max = n
							}
						}
						enumSlice = make([]interface{}, max+1)
						for _, oi := range order {
							k := keys[oi]
							v, _ := e.Get(k)
							n := idx[oi]
							if n >= 0 && n < len(enumSlice) {
								enumSlice[n] = v
							}
						}
					} else {
						enumSlice = make([]interface{}, 0, len(keys))
						for _, k := range keys {
							v, _ := e.Get(k)
							enumSlice = append(enumSlice, v)
						}
					}
				}

				if enumSlice != nil {
					mapped := make([]interface{}, len(dataColumn))
					for i, v := range dataColumn {
						if v == nil {
							mapped[i] = nil
							continue
						}
						idx := -1
						switch t := v.(type) {
						case int:
							idx = t
						case int8:
							idx = int(t)
						case int16:
							idx = int(t)
						case int32:
							idx = int(t)
						case int64:
							idx = int(t)
						case uint:
							idx = int(t)
						case uint8:
							idx = int(t)
						case uint16:
							idx = int(t)
						case uint32:
							idx = int(t)
						case uint64:
							idx = int(t)
						case float32:
							idx = int(t)
						case float64:
							idx = int(t)
						case string:
							if n, err := strconv.Atoi(t); err == nil {
								idx = n
							}
						case json.Number:
							if n, err := strconv.Atoi(string(t)); err == nil {
								idx = n
							}
						default:
						}
						if idx >= 0 && idx < len(enumSlice) {
							mapped[i] = enumSlice[idx]
						} else {
							mapped[i] = v
						}
					}
					columns = append(columns, mapped)
					continue
				}
			}
		}

		columns = append(columns, dataColumn)
	}

	if len(columns) == 0 {
		return []*orderedmap.OrderedMap{}
	}

	numEntries := len(columns[0])
	for _, col := range columns {
		if len(col) < numEntries {
			numEntries = len(col)
		}
	}

	result := make([]*orderedmap.OrderedMap, 0, numEntries)
	for i := 0; i < numEntries; i++ {
		entry := orderedmap.New()
		entry.SetEscapeHTML(false)
		for j, key := range columnLabels {
			if i < len(columns[j]) {
				entry.Set(key, columns[j][i])
			} else {
				entry.Set(key, nil)
			}
		}
		result = append(result, entry)
	}
	return result
}

func NuverseMasterRestorer(masterData *orderedmap.OrderedMap, nuverseStructureFilePath string) (*orderedmap.OrderedMap, error) {
	restoredCompactMaster := orderedmap.New()
	restoredCompactMaster.SetEscapeHTML(false)
	structures, err := loadStructures(nuverseStructureFilePath)
	if err != nil {
		return nil, fmt.Errorf("failed to load nuverve master structure: %v", err)
	}

	for _, key := range masterData.Keys() {
		value, _ := masterData.Get(key)
		if len(key) == 0 {
			continue
		}
		func() {
			defer func() {
				if r := recover(); r != nil {
					panic(fmt.Errorf("error restoring key %s: %v", key, r))
				}
			}()

			if len(key) >= 7 && key[:7] == "compact" {
				if vOm, ok := value.(*orderedmap.OrderedMap); ok {
					data := RestoreCompactData(vOm)
					newKeyOriginal := key[7:]
					if len(newKeyOriginal) > 0 {
						newKey := string(newKeyOriginal[0]+32) + newKeyOriginal[1:]
						restoredCompactMaster.Set(newKey, data)
					}
				}
				return
			}

			var idKey string
			if key == "eventCards" {
				idKey = "cardId"
			}

			if structDefVal, exists := structures.Get(key); exists {
				if arr, ok := value.([]interface{}); ok {
					newArr := make([]*orderedmap.OrderedMap, 0, len(arr))
					if def, ok := structDefVal.([]interface{}); ok {
						for _, v := range arr {
							if subArr, ok := v.([]interface{}); ok {
								newArr = append(newArr, RestoreDict(subArr, def))
							}
						}
						masterData.Set(key, newArr)
						value = any(newArr)
					}
				}
			}

			if idKey != "" {
				arrAny, _ := masterData.Get(key)
				var arr []*orderedmap.OrderedMap
				if a, ok := arrAny.([]*orderedmap.OrderedMap); ok {
					arr = a
				}
				if len(arr) > 0 {
					valueIDs := make(map[any]bool, len(arr))
					for _, item := range arr {
						if id, ok := item.Get(idKey); ok {
							valueIDs[id] = true
						}
					}
					merged := make([]*orderedmap.OrderedMap, 0)
					if vs, ok := value.([]interface{}); ok {
						for _, x := range vs {
							var m *orderedmap.OrderedMap
							switch t := x.(type) {
							case *orderedmap.OrderedMap:
								m = t
							case map[string]any:
								keys := make([]string, 0, len(t))
								for k := range t {
									keys = append(keys, k)
								}
								sort.Strings(keys)
								om := orderedmap.New()
								om.SetEscapeHTML(false)
								for _, k2 := range keys {
									om.Set(k2, t[k2])
								}
								m = om
							}
							if m != nil {
								if id, ok := m.Get(idKey); ok {
									if !valueIDs[id] {
										merged = append(merged, m)
									}
								}
							}
						}
					}
					merged = append(merged, arr...)
					sort.SliceStable(merged, func(i, j int) bool {
						vi, _ := merged[i].Get(idKey)
						vj, _ := merged[j].Get(idKey)
						return toInt64(vi) < toInt64(vj)
					})
					masterData.Set(key, merged)
				}
			}
		}()
	}

	for _, k := range masterData.Keys() {
		v, _ := masterData.Get(k)
		restoredCompactMaster.Set(k, v)
	}

	return restoredCompactMaster, nil
}

func toInt64(v any) int64 {
	switch t := v.(type) {
	case int:
		return int64(t)
	case int32:
		return int64(t)
	case int64:
		return t
	case uint:
		return int64(t)
	case uint32:
		return int64(t)
	case uint64:
		if t > ^uint64(0)>>1 {
			return int64(^uint64(0) >> 1)
		}
		return int64(t)
	case float64:
		return int64(t)
	default:
		return 0
	}
}
