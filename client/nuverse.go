package client

import (
	"fmt"
	"os"
	"sort"

	"github.com/bytedance/sonic"
	"github.com/iancoleman/orderedmap"
)

func loadStructures(path string) (*orderedmap.OrderedMap, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	om := orderedmap.New()
	if err := sonic.Unmarshal(data, om); err != nil {
		return nil, err
	}
	return om, nil
}

func RestoreDict(arrayData []interface{}, keyStructure []interface{}) *orderedmap.OrderedMap {
	result := orderedmap.New()

	for i, key := range keyStructure {
		switch k := key.(type) {
		case string:
			if i < len(arrayData) && arrayData[i] != nil {
				result.Set(k, arrayData[i])
			}
		case []interface{}:
			if len(k) < 2 {
				continue
			}
			keyName, ok := k[0].(string)
			if !ok {
				continue
			}
			switch second := k[1].(type) {
			case []interface{}:
				var subList []*orderedmap.OrderedMap
				if i < len(arrayData) {
					if arr, ok := arrayData[i].([]interface{}); ok {
						for _, sub := range arr {
							if subArr, ok := sub.([]interface{}); ok {
								subList = append(subList, RestoreDict(subArr, second))
							}
						}
					}
				}
				result.Set(keyName, subList)
			case map[string]any:
				if tupleKeysRaw, found := second["__tuple__"]; found {
					if tupleKeys, ok := tupleKeysRaw.([]interface{}); ok {
						dict := orderedmap.New()
						if i < len(arrayData) {
							if arr, ok := arrayData[i].([]interface{}); ok {
								for j, v := range arr {
									if v != nil && j < len(tupleKeys) {
										if keyStr, ok := tupleKeys[j].(string); ok {
											dict.Set(keyStr, v)
										}
									}
								}
							}
						}
						result.Set(keyName, dict)
					}
				}
			}
		}
	}
	return result
}

func RestoreCompactData(data *orderedmap.OrderedMap) []*orderedmap.OrderedMap {
	var columnLabels []string
	var columns [][]interface{}

	var enumRaw any
	if v, ok := data.Get("__ENUM__"); ok {
		enumRaw = v
	}

	for _, key := range data.Keys() {
		if key == "__ENUM__" {
			continue
		}
		val, _ := data.Get(key)
		columnLabels = append(columnLabels, key)
		var dataColumn []interface{}
		if v, ok := val.([]interface{}); ok {
			dataColumn = v
		} else {
			dataColumn = []interface{}{}
		}

		if enumRaw != nil {
			var enumMap map[string]any
			switch em := enumRaw.(type) {
			case *orderedmap.OrderedMap:
				enumMap = make(map[string]any, len(em.Keys()))
				for _, ek := range em.Keys() {
					if ev, ok := em.Get(ek); ok {
						enumMap[ek] = ev
					}
				}
			case map[string]any:
				enumMap = em
			}
			if enumMap != nil {
				if enumColumnRaw, ok := enumMap[key]; ok {
					var enumSlice []interface{}
					if e, ok := enumColumnRaw.([]interface{}); ok {
						enumSlice = e
					}
					if enumSlice != nil {
						columnValues := make([]interface{}, 0, len(dataColumn))
						for _, v := range dataColumn {
							if v == nil {
								columnValues = append(columnValues, nil)
								continue
							}
							var index int
							switch t := v.(type) {
							case int:
								index = t
							case int32:
								index = int(t)
							case int64:
								index = int(t)
							case float64:
								index = int(t)
							default:
								index = 0
							}
							if index >= 0 && index < len(enumSlice) {
								columnValues = append(columnValues, enumSlice[index])
							} else {
								columnValues = append(columnValues, nil)
							}
						}
						columns = append(columns, columnValues)
						continue
					}
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
					var newArr []*orderedmap.OrderedMap
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
					var merged []*orderedmap.OrderedMap
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
