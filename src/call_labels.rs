use std::collections::HashMap;
use std::sync::LazyLock;

static LABELS: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(|| {
    let mut labels = HashMap::new();
    add(
        &mut labels,
        "",
        "len:value;type:value;append:values,value;assert:condition,message;ok:value;err:problem",
    );
    add(
        &mut labels,
        "std.atomics",
        "create:value;load:atomic;store:atomic,value;swap:atomic,value;add:atomic,amount;compare_exchange:atomic,expected,replacement",
    );
    for module in [
        "std.bigint",
        "std.decimal",
        "std.i8",
        "std.i16",
        "std.i32",
        "std.u8",
        "std.u16",
        "std.u32",
        "std.u64",
        "std.i128",
        "std.u128",
    ] {
        add(
            &mut labels,
            module,
            "parse:source;from_int:value;format:value;to_int:value",
        );
    }
    add(
        &mut labels,
        "std.binary",
        "concat:left,right;u16_be:value;u16_le:value;u32_be:value;u32_le:value;u64_be:value;u64_le:value;i16_be:value;i16_le:value;i32_be:value;i32_le:value;int_be:value;int_le:value;float_be:value;float_le:value;read_u16_be:bytes,offset;read_u16_le:bytes,offset;read_u32_be:bytes,offset;read_u32_le:bytes,offset;read_u64_be:bytes,offset;read_u64_le:bytes,offset;read_i16_be:bytes,offset;read_i16_le:bytes,offset;read_i32_be:bytes,offset;read_i32_le:bytes,offset;read_int_be:bytes,offset;read_int_le:bytes,offset;read_float_be:bytes,offset;read_float_le:bytes,offset",
    );
    add(
        &mut labels,
        "std.bytes",
        "from_string:value;from_values:values;to_string:bytes;length:bytes;get:bytes,index;slice:bytes,offset,length",
    );
    add(
        &mut labels,
        "std.channels",
        "create:capacity;send:channel,value,timeout;receive:channel,timeout",
    );
    add(
        &mut labels,
        "std.compression",
        "gzip:bytes,level;gzip_decode:bytes,maximum;zlib:bytes,level;zlib_decode:bytes,maximum",
    );
    add(
        &mut labels,
        "std.crypto",
        "sha256:bytes;hmac_sha256:key,message;hmac_sha256_verify:key,message,expected;random_bytes:length;password_hash:password,salt,memory_kib,iterations,lanes;password_verify:password,encoded;key_import:bytes;key_generate:;encrypt:key,nonce,associated,plaintext;decrypt:key,nonce,associated,ciphertext;ed25519_public:key;ed25519_sign:key,message;ed25519_verify:public_key,message,signature",
    );
    add(
        &mut labels,
        "std.csv",
        "decode:source,headers,delimiter,maximum_rows;encode:rows,headers,delimiter",
    );
    add(
        &mut labels,
        "std.encoding",
        "hex_encode:bytes;hex_decode:source;base64_encode:bytes;base64_decode:source;base64url_encode:bytes;base64url_decode:source",
    );
    add(&mut labels, "std.env", "get:name");
    add(
        &mut labels,
        "std.files",
        "read:path;write:path,contents;exists:path;open_read:path;open_write:path;read_from:file,maximum;write_to:file,contents;close:file",
    );
    add(
        &mut labels,
        "std.files",
        "read_async:path,maximum;write_async:path,contents",
    );
    add(&mut labels, "std.float", "parse:source;format:value");
    add(&mut labels, "std.int", "parse:source;format:value");
    add(
        &mut labels,
        "std.host",
        "invoke:name,request;invoke_async:name,request;open:kind,request;call:handle,name,request;close:handle",
    );
    add(
        &mut labels,
        "std.iter",
        "from:values;range:start,end,step;lines:file,maximum_bytes;tcp_lines:stream,maximum_bytes,timeout;next:iterator;take:iterator,count;skip:iterator,count;transform:iterator,by;select:iterator,by;collect:iterator;chain:left,right;count:iterator;fold:iterator,initial,by;any:iterator,by;every:iterator,by;find:iterator,by",
    );
    add(
        &mut labels,
        "std.json",
        "valid:source;compact:source;pretty:source;parse:source;encode:value;decode:schema,source;read_next:file,maximum;read_next_as:schema,file,maximum",
    );
    add(
        &mut labels,
        "std.list",
        "batch:values,count;transform:values,by;select:values,by;fold:values,initial,by;any:values,by;every:values,by",
    );
    add(
        &mut labels,
        "std.locks",
        "create:value;acquire:lock,timeout;read:guard;write:guard,value;close:guard",
    );
    add(
        &mut labels,
        "std.log",
        "info:message;warn:message;error:message;event:level,message,fields",
    );
    add(
        &mut labels,
        "std.map",
        "of:key,value;set:map,key,value;get:map,key;contains:map,key;remove:map,key;length:map;keys:map;values:map",
    );
    add(
        &mut labels,
        "std.native",
        "open:path;call_int:library,symbol,arguments;call_float:library,symbol,arguments;call_buffer:library,symbol,input,capacity;close:library",
    );
    add(
        &mut labels,
        "std.net",
        "listen:host,port;accept:listener,timeout;connect:host,port,timeout;tls_connect:host,port,timeout,options;read:stream,maximum;read_exact_bytes:stream,length,timeout;read_line:stream,maximum,timeout;write:stream,value;write_some:stream,value,maximum,timeout;wait_ready:stream,interest,timeout;wait_ready_any:streams,interest,timeout;read_ready:stream,maximum,timeout;write_ready:stream,value,chunk_size,timeout;tls_read_exact_bytes:stream,length,timeout;tls_read_line:stream,maximum,timeout;tls_write_ready:stream,value,chunk_size,timeout;tls_close:stream;close:stream",
    );
    add(
        &mut labels,
        "std.path",
        "join:left,right;basename:path;dirname:path",
    );
    add(&mut labels, "std.process", "run:command,arguments");
    add(
        &mut labels,
        "std.reflect",
        "kind:value;fields:value;schema:declaration",
    );
    add(&mut labels, "std.plans", "encode:plan;decode:shape,bytes");
    add(
        &mut labels,
        "std.uint",
        "parse:source;format:value;from_int:value;to_int:value;wrapping_add:left,right;wrapping_sub:left,right;wrapping_mul:left,right;min:;max:",
    );
    add(&mut labels, "std.gpu", "available:;open:adapter");
    add(&mut labels, "std.problems", "create:kind,message");
    add(
        &mut labels,
        "std.source",
        "shape:name,fields,derives;choice:name,cases;binding:name,value;function:name,takes,gives,body;give:expression;call:expression;when:condition,then,otherwise;each:name,in,body",
    );
    add(
        &mut labels,
        "std.set",
        "of:value;add:set,value;contains:set,value;remove:set,value;length:set;values:set",
    );
    add(
        &mut labels,
        "std.tasks",
        "spawn:operation;await:task;await_for:task,timeout;cancel:task;all:tasks;race:tasks",
    );
    add(
        &mut labels,
        "std.text",
        "concat:left,right;split:value,separator,maximum;split_last:value,separator;starts_with:value,prefix;contains:value,needle;ends_with:value,suffix;index_of:value,needle;slice:value,start,end;replace:value,needle,replacement,maximum;trim:value;trim_start:value;trim_end:value;to_upper:value;to_lower:value;join:parts,separator;lines:value;repeat:value,count;pad_start:value,width,pad;pad_end:value,width,pad",
    );
    add(
        &mut labels,
        "std.time",
        "sleep:seconds;now_zoned:zone;from_unix:seconds,zone;parse:source;format:value;in_zone:value,zone;unix:value;add_seconds:value,seconds;monotonic:;year:value;month:value;day:value;hour:value;minute:value;second:value;weekday:value;difference_seconds:left,right",
    );
    add(
        &mut labels,
        "std.transactions",
        "create:map;get:transaction,key;set:transaction,key,value;remove:transaction,key;commit:transaction;rollback:transaction;close:transaction",
    );
    add(
        &mut labels,
        "std.web",
        "encode_component:value;decode_component:value;get:url,timeout;headers:;request:method,url,headers,body,timeout,maximum;read_request:stream,maximum;respond:stream,status,headers,body;websocket_connect:host,port,path,timeout;websocket_secure_connect:host,port,path,timeout,options;websocket_accept:stream,request;websocket_send:websocket,message;websocket_receive:websocket,maximum;websocket_close:websocket;websocket_secure_listen:host,port,certificate_pem,private_key_pem,options;websocket_secure_accept:listener,timeout;tls_options:;tls_close:listener",
    );
    labels
});

fn add(labels: &mut HashMap<String, Vec<String>>, module: &str, specification: &str) {
    for entry in specification.split(';') {
        let (name, parameters) = entry
            .split_once(':')
            .expect("standard label entries contain ':'");
        let parameters = if parameters.is_empty() {
            Vec::new()
        } else {
            parameters.split(',').map(ToString::to_string).collect()
        };
        let path = if module.is_empty() {
            name.to_string()
        } else {
            format!("{module}.{name}")
        };
        labels.insert(path, parameters);
    }
}

pub(crate) fn get(path: &str) -> Option<&'static [String]> {
    LABELS.get(path).map(Vec::as_slice)
}

pub(crate) fn owned() -> HashMap<String, Vec<String>> {
    LABELS.clone()
}
