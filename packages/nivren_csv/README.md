# nivren_csv

Official bounded CSV tables for Nivren Edition 3.

Rows use ordinary `Map<String,String>` values and callers declare the ordered headers, making schema and output order explicit. The parser supports quoted delimiters, quotes, CRLF/LF records, and quoted newlines. Inputs and outputs are capped at 16 MiB, fields at 1 MiB, columns at 4,096, and rows at a caller-selected limit no greater than one million.

`decode_with` and `encode_with` accept a visible single-byte ASCII delimiter. `read` and `write` keep filesystem authority explicit through `FileRead` and `FileWrite`.
