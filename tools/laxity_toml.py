"""The repository's TOML subset for interpreters without tomllib or tomli."""

import math
import re


# Accept bare keys, dotted table headers, top-level arrays of tables, unescaped double-quoted strings, unsigned decimal/hex integers, unsigned decimal floats, booleans, scalar arrays (including multiline and trailing commas), single-line inline tables with scalar/array values, and comments; refuse all other TOML constructs, duplicate definitions and table/value conflicts so an unfamiliar value cannot be mistaken for a supported one.
KEY = r"[A-Za-z0-9_-]+"
TOKEN = re.compile(r'"[^"\\]*"|[\[\]{},=]|[^ \t"\[\]{},=#]+')


def loads(text):
    tokens = []
    lines = text.split("\n")
    for line_number, line in enumerate(lines, 1):
        if line.endswith("\r") and line_number < len(lines):
            line = line[:-1]
        if any((ord(ch) < 32 and ch != "\t") or ord(ch) == 127 or
               0xD800 <= ord(ch) <= 0xDFFF for ch in line):
            raise ValueError("line %d: invalid control character or Unicode scalar" % line_number)
        offset = 0
        while offset < len(line):
            if line[offset] in " \t":
                offset += 1
                continue
            if line[offset] == "#":
                break
            match = TOKEN.match(line, offset)
            if not match:
                raise ValueError("line %d: unsupported string or character" % line_number)
            tokens.append((match.group(), line_number))
            offset = match.end()
        tokens.append(("\n", line_number))
    tokens.append(("", len(lines)))
    position = 0

    def fail(message):
        raise ValueError("line %d: %s" % (tokens[position][1], message))

    def take(expected=None):
        nonlocal position
        token = tokens[position][0]
        if expected is not None and token != expected:
            fail("expected %r, found %r" % (expected, token))
        if not token:
            fail("unexpected end of document")
        position += 1
        return token

    def newlines():
        while tokens[position][0] == "\n":
            take()

    def key():
        if not re.fullmatch(KEY, tokens[position][0]):
            fail("expected a bare key")
        return take()

    def scalar():
        token = tokens[position][0]
        if token.startswith('"'):
            result = token[1:-1]
        elif token in ("true", "false"):
            result = token == "true"
        elif re.fullmatch(r"0|[1-9][0-9]*|0x[0-9a-fA-F]+", token):
            try:
                result = int(token, 16 if token.startswith("0x") else 10)
            except ValueError as err:
                fail(str(err))
        elif re.fullmatch(r"(?:0|[1-9][0-9]*)\.[0-9]+", token):
            result = float(token)
            if not math.isfinite(result):
                fail("float is outside the finite range")
        else:
            fail("unsupported value %r" % token)
        take()
        return result

    def assignment(table, inline=False):
        name = key()
        if name in table:
            fail("duplicate key %r" % name)
        take("=")
        table[name] = value(inline)

    def value(inline=False):
        if tokens[position][0] == "[":
            take("[")
            result = []
            if not inline:
                newlines()
            while tokens[position][0] != "]":
                result.append(scalar())
                if not inline:
                    newlines()
                if tokens[position][0] != ",":
                    break
                take(",")
                if not inline:
                    newlines()
            take("]")
            return result
        if tokens[position][0] == "{" and not inline:
            take("{")
            result = {}
            if tokens[position][0] != "}":
                assignment(result, inline=True)
                while tokens[position][0] == ",":
                    take(",")
                    assignment(result, inline=True)
            take("}")
            return result
        return scalar()

    root = current = {}
    tables = {(): "table"}
    newlines()
    while tokens[position][0]:
        if tokens[position][0] == "[":
            take("[")
            array = tokens[position][0] == "["
            if array:
                take("[")
            header = tokens[position][0]
            if not re.fullmatch(KEY + r"(?:\." + KEY + r")*", header):
                fail("expected a dotted bare table name")
            path = tuple(header.split("."))
            if array and len(path) != 1:
                fail("only top-level arrays of tables are supported")
            parent = root
            for index, name in enumerate(path[:-1], 1):
                prefix = path[:index]
                if name not in parent:
                    parent[name] = {}
                    tables[prefix] = "implicit"
                if tables.get(prefix) not in ("implicit", "table"):
                    fail("table conflicts with a value or array of tables")
                parent = parent[name]
            name = path[-1]
            kind = tables.get(path)
            if array:
                if name in parent and kind != "array":
                    fail("array of tables conflicts with an existing definition")
                parent.setdefault(name, []).append({})
                current = parent[name][-1]
                tables[path] = "array"
            else:
                if name in parent and kind != "implicit":
                    fail("duplicate table or table/value conflict")
                current = parent.setdefault(name, {})
                tables[path] = "table"
            take()
            take("]")
            if array:
                take("]")
        else:
            assignment(current)
        take("\n")
        newlines()
    return root
