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

func extractTupleKeys(v interface{}) []interface{} {
	switch s := v.(type) {
	case *orderedmap.OrderedMap:
		if tupleKeysRaw, found := s.Get("__tuple__"); found {
			if keys, ok := tupleKeysRaw.([]interface{}); ok {
				return keys
			}
		}
	case orderedmap.OrderedMap:
		if tupleKeysRaw, found := s.Get("__tuple__"); found {
			if keys, ok := tupleKeysRaw.([]interface{}); ok {
				return keys
			}
		}
	case map[string]interface{}:
		if tupleKeysRaw, found := s["__tuple__"]; found {
			if keys, ok := tupleKeysRaw.([]interface{}); ok {
				return keys
			}
		}
	}
	return nil
}

func buildDictFromTuple(tupleKeys []interface{}, tupleVals []interface{}) *orderedmap.OrderedMap {
	dict := orderedmap.New()
	dict.SetEscapeHTML(false)

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
	return dict
}

func handleSimpleTuple(keyStructure []interface{}, arrayData []interface{}) (*orderedmap.OrderedMap, bool) {
	if len(keyStructure) != 2 {
		return nil, false
	}

	keyName, ok := keyStructure[0].(string)
	if !ok {
		return nil, false
	}

	tupleKeys := extractTupleKeys(keyStructure[1])
	if tupleKeys == nil {
		return nil, false
	}

	tupleVals := arrayData
	if len(arrayData) == 1 {
		if innerArr, ok := arrayData[0].([]interface{}); ok {
			tupleVals = innerArr
		}
	}

	result := orderedmap.New()
	result.SetEscapeHTML(false)
	dict := buildDictFromTuple(tupleKeys, tupleVals)
	result.Set(keyName, dict)
	return result, true
}

func processTupleField(second interface{}, arrayData []interface{}, i int) *orderedmap.OrderedMap {
	tupleKeys := extractTupleKeys(second)
	if tupleKeys == nil {
		return nil
	}

	var tupleVals []interface{}
	if i < len(arrayData) && arrayData[i] != nil {
		tupleVals, _ = arrayData[i].([]interface{})
	}

	if tupleVals == nil {
		return nil
	}

	return buildDictFromTuple(tupleKeys, tupleVals)
}

func processNestedArray(arrayData []interface{}, i int, second []interface{}) []*orderedmap.OrderedMap {
	subList := make([]*orderedmap.OrderedMap, 0)
	if i >= len(arrayData) {
		return subList
	}

	arr, ok := arrayData[i].([]interface{})
	if !ok {
		return subList
	}

	for _, sub := range arr {
		subArr, ok := sub.([]interface{})
		if !ok {
			continue
		}

		if len(second) > 0 {
			if innerStruct, ok := second[0].([]interface{}); ok && len(innerStruct) >= 2 {
				subList = append(subList, RestoreDict(subArr, innerStruct))
			} else {
				subList = append(subList, RestoreDict(subArr, second))
			}
		} else {
			subList = append(subList, RestoreDict(subArr, second))
		}
	}
	return subList
}

func RestoreDict(arrayData []interface{}, keyStructure []interface{}) *orderedmap.OrderedMap {
	result := orderedmap.New()
	result.SetEscapeHTML(false)
	if simpleResult, ok := handleSimpleTuple(keyStructure, arrayData); ok {
		return simpleResult
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
				if dict := processTupleField(second, arrayData, i); dict != nil {
					result.Set(keyName, dict)
				}

			case []interface{}:
				subList := processNestedArray(arrayData, i, second)
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

func extractEnumOM(data *orderedmap.OrderedMap) *orderedmap.OrderedMap {
	v, ok := data.Get("__ENUM__")
	if !ok {
		return nil
	}

	switch em := v.(type) {
	case *orderedmap.OrderedMap:
		return em
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
		return om
	}
	return nil
}

func convertEnumToSlice(e *orderedmap.OrderedMap) []interface{} {
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
		return convertNumericEnumToSlice(e, keys, idx)
	}
	return convertStringEnumToSlice(e, keys)
}

func convertNumericEnumToSlice(e *orderedmap.OrderedMap, keys []string, idx []int) []interface{} {
	order := make([]int, len(keys))
	for i := range order {
		order[i] = i
	}
	sort.Slice(order, func(i, j int) bool { return idx[order[i]] < idx[order[j]] })

	kMax := -1
	for _, n := range idx {
		if n > kMax {
			kMax = n
		}
	}

	enumSlice := make([]interface{}, kMax+1)
	for _, oi := range order {
		k := keys[oi]
		v, _ := e.Get(k)
		n := idx[oi]
		if n >= 0 && n < len(enumSlice) {
			enumSlice[n] = v
		}
	}
	return enumSlice
}

func convertStringEnumToSlice(e *orderedmap.OrderedMap, keys []string) []interface{} {
	enumSlice := make([]interface{}, 0, len(keys))
	for _, k := range keys {
		v, _ := e.Get(k)
		enumSlice = append(enumSlice, v)
	}
	return enumSlice
}

func convertIntType(v interface{}) (int, bool) {
	switch t := v.(type) {
	case int:
		return t, true
	case int8:
		return int(t), true
	case int16:
		return int(t), true
	case int32:
		return int(t), true
	case int64:
		return int(t), true
	case uint:
		return int(t), true
	case uint8:
		return int(t), true
	case uint16:
		return int(t), true
	case uint32:
		return int(t), true
	case uint64:
		return int(t), true
	}
	return 0, false
}

func convertFloatType(v interface{}) (int, bool) {
	switch t := v.(type) {
	case float32:
		return int(t), true
	case float64:
		return int(t), true
	}
	return 0, false
}

func convertStringType(v interface{}) (int, bool) {
	switch t := v.(type) {
	case string:
		if n, err := strconv.Atoi(t); err == nil {
			return n, true
		}
	case json.Number:
		if n, err := strconv.Atoi(string(t)); err == nil {
			return n, true
		}
	}
	return 0, false
}

func convertValueToIndex(v interface{}) int {
	if idx, ok := convertIntType(v); ok {
		return idx
	}
	if idx, ok := convertFloatType(v); ok {
		return idx
	}
	if idx, ok := convertStringType(v); ok {
		return idx
	}
	return -1
}

func mapEnumValue(v interface{}, enumSlice []interface{}) interface{} {
	if v == nil {
		return nil
	}

	idx := convertValueToIndex(v)
	if idx >= 0 && idx < len(enumSlice) {
		return enumSlice[idx]
	}
	return v
}

func processEnumColumn(dataColumn []interface{}, enumColRaw interface{}) []interface{} {
	var enumSlice []interface{}

	switch e := enumColRaw.(type) {
	case []interface{}:
		enumSlice = e
	case *orderedmap.OrderedMap:
		enumSlice = convertEnumToSlice(e)
	}

	if enumSlice == nil {
		return dataColumn
	}

	mapped := make([]interface{}, len(dataColumn))
	for i, v := range dataColumn {
		mapped[i] = mapEnumValue(v, enumSlice)
	}
	return mapped
}

func RestoreCompactData(data *orderedmap.OrderedMap) []*orderedmap.OrderedMap {
	var (
		columnLabels []string
		columns      [][]interface{}
	)

	enumOM := extractEnumOM(data)

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
				dataColumn = processEnumColumn(dataColumn, enumColRaw)
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

func restoreStructuredData(key string, value any, structures *orderedmap.OrderedMap, masterData *orderedmap.OrderedMap) any {
	structDefVal, exists := structures.Get(key)
	if !exists {
		return value
	}

	arr, ok := value.([]interface{})
	if !ok {
		return value
	}

	def, ok := structDefVal.([]interface{})
	if !ok {
		return value
	}

	newArr := make([]*orderedmap.OrderedMap, 0, len(arr))
	for _, v := range arr {
		if subArr, ok := v.([]interface{}); ok {
			newArr = append(newArr, RestoreDict(subArr, def))
		}
	}

	// Only update masterData if we successfully restored some elements
	// If newArr is empty but original arr was not, keep the original value
	if len(newArr) == 0 && len(arr) > 0 {
		return value
	}

	masterData.Set(key, newArr)
	return any(newArr)
}

func extractValueIDs(arr []*orderedmap.OrderedMap, idKey string) map[any]bool {
	valueIDs := make(map[any]bool, len(arr))
	for _, item := range arr {
		if id, ok := item.Get(idKey); ok {
			valueIDs[id] = true
		}
	}
	return valueIDs
}

func convertToOrderedMap(x any) *orderedmap.OrderedMap {
	switch t := x.(type) {
	case *orderedmap.OrderedMap:
		return t
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
		return om
	}
	return nil
}

func mergeAndSortData(value any, arr []*orderedmap.OrderedMap, valueIDs map[any]bool, idKey string) []*orderedmap.OrderedMap {
	merged := make([]*orderedmap.OrderedMap, 0)
	if vs, ok := value.([]interface{}); ok {
		for _, x := range vs {
			m := convertToOrderedMap(x)
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
	return merged
}

func handleIDMerging(key string, value any, idKey string, masterData *orderedmap.OrderedMap) {
	if idKey == "" {
		return
	}

	arrAny, _ := masterData.Get(key)
	var arr []*orderedmap.OrderedMap
	if a, ok := arrAny.([]*orderedmap.OrderedMap); ok {
		arr = a
	}
	if len(arr) == 0 {
		return
	}

	valueIDs := extractValueIDs(arr, idKey)
	merged := mergeAndSortData(value, arr, valueIDs, idKey)
	masterData.Set(key, merged)
}

func NuverseMasterRestorer(masterData *orderedmap.OrderedMap, nuverseStructureFilePath string) (*orderedmap.OrderedMap, error) {
	restoredCompactMaster := orderedmap.New()
	restoredCompactMaster.SetEscapeHTML(false)
	structures, err := loadStructures(nuverseStructureFilePath)
	if err != nil {
		return nil, fmt.Errorf("failed to load nuverve master structure: %v", err)
	}
	restoredFromCompact := make(map[string]bool)
	masterDataKeys := masterData.Keys()
	for _, key := range masterDataKeys {
		value, _ := masterData.Get(key)
		if len(key) == 0 {
			continue
		}
		if len(key) < 7 || key[:7] != "compact" {
			continue
		}
		func() {
			defer func() {
				if r := recover(); r != nil {
					panic(fmt.Errorf("error restoring key %s: %v", key, r))
				}
			}()
			restoredCompactMaster.Set(key, value)
			if vOm, ok := value.(*orderedmap.OrderedMap); ok {
				data := RestoreCompactData(vOm)
				newKeyOriginal := key[7:]
				if len(newKeyOriginal) > 0 {
					newKey := string(newKeyOriginal[0]+32) + newKeyOriginal[1:]
					var structuredData any = data
					var idKey string
					if newKey == "eventCards" {
						idKey = "cardId"
					}
					if idKey != "" {
						masterData.Set(newKey, data)
						handleIDMerging(newKey, structuredData, idKey, masterData)
						structuredData, _ = masterData.Get(newKey)
					}
					restoredCompactMaster.Set(newKey, structuredData)
					restoredFromCompact[newKey] = true
				}
			}
		}()
	}
	for _, key := range masterDataKeys {
		value, _ := masterData.Get(key)
		if len(key) == 0 {
			continue
		}
		if len(key) >= 7 && key[:7] == "compact" {
			continue
		}
		if restoredFromCompact[key] {
			continue
		}
		func() {
			defer func() {
				if r := recover(); r != nil {
					panic(fmt.Errorf("error restoring key %s: %v", key, r))
				}
			}()
			var idKey string
			if key == "eventCards" {
				idKey = "cardId"
			}
			restoredValue := restoreStructuredData(key, value, structures, masterData)
			handleIDMerging(key, restoredValue, idKey, masterData)
			finalValue, exists := masterData.Get(key)
			if exists {
				restoredCompactMaster.Set(key, finalValue)
			} else {
				restoredCompactMaster.Set(key, restoredValue)
			}
		}()
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
