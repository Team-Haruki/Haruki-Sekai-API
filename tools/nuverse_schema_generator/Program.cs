using System.Text.Json;
using System.Text.Json.Nodes;
using Mono.Cecil;

const string defaultDummyDll = "/Users/seiun/Desktop/pjskida/cn/DummyDll";
const string defaultOutput = "../../Data/structures/nuverse_schema_bundle.json";

var dummyDll = args.Length > 0 ? args[0] : defaultDummyDll;
var output = args.Length > 1 ? args[1] : defaultOutput;
var assemblyPath = Path.Combine(dummyDll, "Assembly-CSharp.dll");
if (!File.Exists(assemblyPath))
{
    Console.Error.WriteLine($"Assembly-CSharp.dll not found: {assemblyPath}");
    return 1;
}

var resolver = new DefaultAssemblyResolver();
resolver.AddSearchDirectory(dummyDll);
var readerParameters = new ReaderParameters { AssemblyResolver = resolver };
var assembly = AssemblyDefinition.ReadAssembly(assemblyPath, readerParameters);

var allTypes = assembly.MainModule.Types
    .SelectMany(FlattenTypes)
    .Where(t => t.FullName.StartsWith("Sekai.", StringComparison.Ordinal))
    .Where(HasMessagePackObject)
    .ToDictionary(t => t.FullName, t => t);

var roots = allTypes.Values
    .Where(t => t.FullName.StartsWith("Sekai.Master", StringComparison.Ordinal)
        || t.FullName.StartsWith("Sekai.User", StringComparison.Ordinal))
    .OrderBy(t => t.FullName, StringComparer.Ordinal)
    .ToList();

var generator = new SchemaGenerator(allTypes);
foreach (var root in roots)
{
    generator.Include(root);
}

var schemas = generator.Schemas
    .OrderBy(node => QualifiedName(node), StringComparer.Ordinal)
    .ToArray();

var master = roots
    .Where(t => t.FullName.StartsWith("Sekai.Master", StringComparison.Ordinal))
    .Select(t => new KeyValuePair<string, string>(MasterKey(t.Name), t.FullName))
    .Where(kv => !string.IsNullOrWhiteSpace(kv.Key))
    .GroupBy(kv => kv.Key, StringComparer.Ordinal)
    .ToDictionary(g => g.Key, g => g.First().Value, StringComparer.Ordinal);

var api = new JsonArray
{
    ApiMapping("/user/{userId}/{targetUserId}/profile", new []
    {
        FieldMapping("userHonors[]", "Sekai.UserHonor"),
        FieldMapping("userProfileHonors[]", "Sekai.UserProfileHonor")
    }),
    ApiMapping("/user/{userId}/event/{eventId}/ranking", new []
    {
        FieldMapping("rankings[].userCard", "Sekai.UserCard"),
        FieldMapping("userWorldBloomChapterRankings[].rankings[].userCard", "Sekai.UserCard")
    }),
    ApiMapping("/event/{eventId}/ranking-border", new []
    {
        FieldMapping("borderRankings[].userCard", "Sekai.UserCard"),
        FieldMapping("userWorldBloomChapterRankingBorders[].borderRankings[].userCard", "Sekai.UserCard")
    })
};

var bundle = new JsonObject
{
    ["schemas"] = new JsonArray(schemas.Select(n => n.DeepClone()).ToArray()),
    ["master"] = JsonSerializer.SerializeToNode(master, JsonOptions())!,
    ["api"] = api
};

Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(output))!);
File.WriteAllText(output, bundle.ToJsonString(JsonOptions()) + Environment.NewLine);
Console.WriteLine($"Wrote {output}");
Console.WriteLine($"Schemas: {schemas.Length}");
Console.WriteLine($"Master mappings: {master.Count}");
Console.WriteLine($"API mappings: {api.Count}");
return 0;

static IEnumerable<TypeDefinition> FlattenTypes(TypeDefinition type)
{
    yield return type;
    foreach (var nested in type.NestedTypes.SelectMany(FlattenTypes))
    {
        yield return nested;
    }
}

static bool HasMessagePackObject(TypeDefinition type)
{
    return type.CustomAttributes.Any(a => a.AttributeType.FullName == "MessagePack.MessagePackObjectAttribute");
}

static string MasterKey(string name)
{
    var bare = name.StartsWith("Master", StringComparison.Ordinal) ? name["Master".Length..] : name;
    if (string.IsNullOrEmpty(bare))
    {
        return "";
    }
    return char.ToLowerInvariant(bare[0]) + bare[1..] + "s";
}

static JsonObject ApiMapping(string path, IEnumerable<JsonObject> fields)
{
    return new JsonObject
    {
        ["path"] = path,
        ["fields"] = new JsonArray(fields.Select(f => f.DeepClone()).ToArray())
    };
}

static JsonObject FieldMapping(string selector, string schema)
{
    return new JsonObject
    {
        ["selector"] = selector,
        ["schema"] = schema
    };
}

static string QualifiedName(JsonNode node)
{
    var obj = node.AsObject();
    var name = obj["name"]?.GetValue<string>() ?? "";
    var ns = obj["namespace"]?.GetValue<string>() ?? "";
    return string.IsNullOrEmpty(ns) ? name : $"{ns}.{name}";
}

static JsonSerializerOptions JsonOptions()
{
    return new JsonSerializerOptions
    {
        WriteIndented = true,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping
    };
}

sealed class SchemaGenerator
{
    private readonly Dictionary<string, TypeDefinition> _types;
    private readonly Dictionary<string, JsonObject> _schemas = new(StringComparer.Ordinal);
    private readonly HashSet<string> _visiting = new(StringComparer.Ordinal);

    public SchemaGenerator(Dictionary<string, TypeDefinition> types)
    {
        _types = types;
    }

    public IEnumerable<JsonObject> Schemas => _schemas.Values;

    public void Include(TypeDefinition type)
    {
        if (_schemas.ContainsKey(type.FullName) || !_visiting.Add(type.FullName))
        {
            return;
        }

        var schema = new JsonObject
        {
            ["type"] = "record",
            ["name"] = type.Name,
            ["namespace"] = NamespaceOf(type),
        };
        _schemas[type.FullName] = schema;

        var fields = new JsonArray();
        foreach (var field in MessagePackFields(type))
        {
            fields.Add(FieldSchema(field));
        }
        schema["fields"] = fields;
        _visiting.Remove(type.FullName);
    }

    private JsonObject FieldSchema(FieldDefinition field)
    {
        var key = MessagePackKey(field.CustomAttributes);
        var type = TypeSchema(field.FieldType, IsNullable(field));
        var obj = new JsonObject
        {
            ["name"] = field.Name,
            ["type"] = type
        };
        if (key.IntKey is { } intKey)
        {
            obj["msgpack_key"] = intKey;
        }
        else
        {
            obj["msgpack_key"] = key.StringKey ?? field.Name;
        }
        return obj;
    }

    private JsonNode TypeSchema(TypeReference type, bool nullable)
    {
        var schema = NonNullableTypeSchema(type);
        if (!nullable)
        {
            return schema;
        }
        return new JsonArray("null", schema);
    }

    private JsonNode NonNullableTypeSchema(TypeReference type)
    {
        if (type is ArrayType arrayType)
        {
            return new JsonObject
            {
                ["type"] = "array",
                ["items"] = NonNullableTypeSchema(arrayType.ElementType)
            };
        }
        if (type is GenericInstanceType generic)
        {
            var fullName = generic.ElementType.FullName;
            if (fullName == "System.Nullable`1")
            {
                return TypeSchema(generic.GenericArguments[0], true);
            }
            if (fullName is "System.Collections.Generic.List`1"
                or "System.Collections.Generic.IList`1"
                or "System.Collections.Generic.IReadOnlyList`1"
                or "System.Collections.ObjectModel.ReadOnlyCollection`1")
            {
                return new JsonObject
                {
                    ["type"] = "array",
                    ["items"] = NonNullableTypeSchema(generic.GenericArguments[0])
                };
            }
            if (fullName is "System.Collections.Generic.Dictionary`2"
                or "System.Collections.Generic.IDictionary`2"
                or "System.Collections.Generic.IReadOnlyDictionary`2")
            {
                return new JsonObject
                {
                    ["type"] = "map",
                    ["values"] = NonNullableTypeSchema(generic.GenericArguments[1])
                };
            }
        }

        return type.FullName switch
        {
            "System.Boolean" => JsonValue.Create("boolean")!,
            "System.Byte" => JsonValue.Create("int")!,
            "System.SByte" => JsonValue.Create("int")!,
            "System.Int16" => JsonValue.Create("int")!,
            "System.UInt16" => JsonValue.Create("int")!,
            "System.Int32" => JsonValue.Create("int")!,
            "System.UInt32" => JsonValue.Create("long")!,
            "System.Int64" => JsonValue.Create("long")!,
            "System.UInt64" => JsonValue.Create("long")!,
            "System.Single" => JsonValue.Create("float")!,
            "System.Double" => JsonValue.Create("double")!,
            "System.String" => JsonValue.Create("string")!,
            "System.DateTime" => JsonValue.Create("long")!,
            _ => ReferenceType(type)
        };
    }

    private JsonNode ReferenceType(TypeReference type)
    {
        var resolved = Resolve(type);
        if (resolved != null && _types.ContainsKey(resolved.FullName))
        {
            Include(resolved);
            return JsonValue.Create(resolved.FullName)!;
        }
        return JsonValue.Create("string")!;
    }

    private TypeDefinition? Resolve(TypeReference type)
    {
        try
        {
            return type.Resolve();
        }
        catch
        {
            return _types.GetValueOrDefault(type.FullName);
        }
    }

    private static string NamespaceOf(TypeDefinition type)
    {
        if (!string.IsNullOrEmpty(type.Namespace))
        {
            return type.Namespace;
        }
        var idx = type.FullName.LastIndexOf('.');
        return idx > 0 ? type.FullName[..idx] : "";
    }

    private static IEnumerable<FieldDefinition> MessagePackFields(TypeDefinition type)
    {
        return type.Fields
            .Where(f => !f.IsStatic)
            .Where(f => !HasAttribute(f.CustomAttributes, "MessagePack.IgnoreMemberAttribute"))
            .Select(f => new { Field = f, Key = MessagePackKey(f.CustomAttributes) })
            .Where(x => x.Key.HasKey)
            .OrderBy(x => x.Key.IntKey ?? int.MaxValue)
            .ThenBy(x => x.Key.StringKey ?? x.Field.Name, StringComparer.Ordinal)
            .Select(x => x.Field);
    }

    private static bool IsNullable(FieldDefinition field)
    {
        if (!field.FieldType.IsValueType)
        {
            return HasAttribute(field.CustomAttributes, "MessagePack.NullableAttribute")
                || HasAttribute(field.CustomAttributes, "JetBrains.Annotations.CanBeNullAttribute");
        }
        if (field.FieldType is GenericInstanceType generic && generic.ElementType.FullName == "System.Nullable`1")
        {
            return true;
        }
        return HasAttribute(field.CustomAttributes, "MessagePack.NullableAttribute")
            || HasAttribute(field.CustomAttributes, "JetBrains.Annotations.CanBeNullAttribute");
    }

    private static bool HasAttribute(IEnumerable<CustomAttribute> attributes, string fullName)
    {
        return attributes.Any(a => a.AttributeType.FullName == fullName);
    }

    private static MsgpackKey MessagePackKey(IEnumerable<CustomAttribute> attributes)
    {
        var attr = attributes.FirstOrDefault(a => a.AttributeType.FullName == "MessagePack.KeyAttribute");
        if (attr == null || attr.ConstructorArguments.Count == 0)
        {
            return default;
        }
        var value = attr.ConstructorArguments[0].Value;
        return value switch
        {
            int i => new MsgpackKey(true, i, null),
            string s => new MsgpackKey(true, null, s),
            _ => default
        };
    }
}

readonly record struct MsgpackKey(bool HasKey, int? IntKey, string? StringKey);
