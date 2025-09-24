package client

import (
	"fmt"
	"os"
	"sort"

	"github.com/bytedance/sonic"
)

func LoadStructures(path string) (map[string][]interface{}, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var structures map[string][]interface{}
	if err := sonic.Unmarshal(data, &structures); err != nil {
		return nil, err
	}
	return structures, nil
}

// RestoreDict converts array_data to a map based on key_structure
func RestoreDict(arrayData []interface{}, keyStructure []interface{}) map[string]interface{} {
	result := make(map[string]interface{})

	for i, key := range keyStructure {
		switch k := key.(type) {
		case string:
			// key is string
			if i < len(arrayData) && arrayData[i] != nil {
				result[k] = arrayData[i]
			}
		case []interface{}:
			// key is list, must check its second element
			if len(k) < 2 {
				continue
			}
			keyName, ok := k[0].(string)
			if !ok {
				continue
			}

			switch second := k[1].(type) {
			case []interface{}:
				// nested list
				var subList []map[string]interface{}
				if i < len(arrayData) {
					if arr, ok := arrayData[i].([]interface{}); ok {
						for _, sub := range arr {
							if subArr, ok := sub.([]interface{}); ok {
								subList = append(subList, RestoreDict(subArr, second))
							}
						}
					}
				}
				result[keyName] = subList

			case map[string]interface{}:
				// check for tuple scheme 1
				if tupleKeysRaw, found := second["__tuple__"]; found {
					if tupleKeys, ok := tupleKeysRaw.([]interface{}); ok {
						dict := make(map[string]interface{})
						if i < len(arrayData) {
							if arr, ok := arrayData[i].([]interface{}); ok {
								for j, v := range arr {
									if v != nil && j < len(tupleKeys) {
										if keyStr, ok := tupleKeys[j].(string); ok {
											dict[keyStr] = v
										}
									}
								}
							}
						}
						result[keyName] = dict
					}
				}
			}
		}
	}

	return result
}

// RestoreCompactData converts compact data into original structure
func RestoreCompactData(data map[string]interface{}) []map[string]interface{} {
	enum, _ := data["__ENUM__"].(map[string]interface{})
	var columnLabels []string
	var columns [][]interface{}

	for column, raw := range data {
		if column == "__ENUM__" {
			continue
		}
		columnLabels = append(columnLabels, column)

		rawArr, _ := raw.([]interface{})
		if e, ok := enum[column].([]interface{}); ok {
			// map enum indices
			var col []interface{}
			for _, idx := range rawArr {
				if idx == nil {
					col = append(col, nil)
				} else {
					if i, ok := idx.(float64); ok && int(i) < len(e) {
						col = append(col, e[int(i)])
					} else {
						col = append(col, nil)
					}
				}
			}
			columns = append(columns, col)
		} else {
			columns = append(columns, rawArr)
		}
	}

	// calculate num_entries
	numEntries := len(columns[0])
	for _, col := range columns {
		if len(col) < numEntries {
			numEntries = len(col)
		}
	}

	var result []map[string]interface{}
	for i := 0; i < numEntries; i++ {
		entry := make(map[string]interface{})
		for j, label := range columnLabels {
			entry[label] = columns[j][i]
		}
		result = append(result, entry)
	}

	return result
}

// NuverseMasterRestorer restores master data
func NuverseMasterRestorer(masterData map[string]interface{}, nuverseStructureFilePath string) (map[string]interface{}, error) {
	restoredCompactMaster := make(map[string]interface{})
	structures, err := LoadStructures(nuverseStructureFilePath)
	if err != nil {
		return nil, fmt.Errorf("failed to load nuverve master structure: %v", err)
	}

	for key, value := range masterData {
		if len(key) > 0 {
			func() {
				defer func() {
					if r := recover(); r != nil {
						panic(fmt.Errorf("error restoring key %s: %v", key, r))
					}
				}()

				if len(key) >= 7 && key[:7] == "compact" {
					if v, ok := value.(map[string]interface{}); ok {
						data := RestoreCompactData(v)
						newKeyOriginal := key[7:]
						if len(newKeyOriginal) > 0 {
							newKey := string(newKeyOriginal[0]+32) + newKeyOriginal[1:]
							restoredCompactMaster[newKey] = data
						}
					}
					return
				}

				var idKey string
				if key == "eventCards" {
					idKey = "cardId"
				}

				if structDef, exists := structures[key]; exists {
					if arr, ok := value.([]interface{}); ok {
						var newArr []map[string]interface{}
						for _, v := range arr {
							if subArr, ok := v.([]interface{}); ok {
								newArr = append(newArr, RestoreDict(subArr, structDef))
							}
						}
						masterData[key] = newArr
					}
				}

				if idKey != "" {
					if arr, ok := masterData[key].([]map[string]interface{}); ok {
						valueIDs := make(map[interface{}]bool)
						for _, item := range arr {
							if id, ok := item[idKey]; ok {
								valueIDs[id] = true
							}
						}
						if valArr, ok := value.([]map[string]interface{}); ok {
							var merged []map[string]interface{}
							for _, x := range valArr {
								if id, ok := x[idKey]; ok {
									if !valueIDs[id] {
										merged = append(merged, x)
									}
								}
							}
							merged = append(merged, arr...)
							sort.Slice(merged, func(i, j int) bool {
								idi := merged[i][idKey].(int)
								idj := merged[j][idKey].(int)
								return idi < idj
							})
							masterData[key] = merged
						}
					}
				}
			}()
		}
	}

	for k, v := range masterData {
		restoredCompactMaster[k] = v
	}

	return restoredCompactMaster, nil
}
