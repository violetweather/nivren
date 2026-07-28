use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, UserFuncName, types};
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntOp {
    Constant(i64),
    Load(u32),
    Define(u32),
    Store(u32),
    Pop,
    Add,
    Subtract,
    Multiply,
    Negate,
    Return,
}

pub struct CompiledFunction {
    _module: JITModule,
    function: unsafe extern "C" fn(*const i64, *mut u8) -> i64,
    parameters: usize,
}

pub struct AotObject;

/// Runtime helper invoked by a complete-program native trace. Returning a
/// non-negative value selects the next instruction. Negative values terminate
/// the trace and are preserved for the runtime (`-1` complete, `-2` return,
/// `-3` checked error).
pub type TraceCallback = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> i64;

/// A Cranelift-compiled control trace for an arbitrary verified bytecode chunk.
/// Individual value operations use the runtime helper ABI, while instruction
/// selection, jumps, loops, and termination execute as native control flow.
pub struct CompiledTrace {
    _module: JITModule,
    function: unsafe extern "C" fn(*mut std::ffi::c_void, TraceCallback) -> i64,
    instructions: usize,
}

pub struct TraceObject;

// SAFETY: See `CompiledFunction`; a trace also owns a finalized immutable JIT
// module and receives all mutable execution state through its caller context.
unsafe impl Send for CompiledTrace {}
// SAFETY: Calls execute immutable code and use caller-owned context values.
unsafe impl Sync for CompiledTrace {}

impl CompiledTrace {
    pub fn compile(instructions: usize) -> Result<Self, String> {
        check_trace_limits(instructions)?;
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "false")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = JITBuilder::with_isa(isa, default_libcall_names());
        let mut module = JITModule::new(builder);
        let signature = trace_signature(&mut module);
        let function_id = module
            .declare_function("nivren_trace", Linkage::Local, &signature)
            .map_err(|error| error.to_string())?;
        let mut context = module.make_context();
        context.func.signature = signature;
        context.func.name = UserFuncName::user(0, 1);
        define_trace_body(&mut context.func, instructions)?;
        module
            .define_function(function_id, &mut context)
            .map_err(|error| error.to_string())?;
        module.clear_context(&mut context);
        module
            .finalize_definitions()
            .map_err(|error| error.to_string())?;
        let pointer = module.get_finalized_function(function_id);
        let function = unsafe {
            std::mem::transmute::<
                *const u8,
                unsafe extern "C" fn(*mut std::ffi::c_void, TraceCallback) -> i64,
            >(pointer)
        };
        Ok(Self {
            _module: module,
            function,
            instructions,
        })
    }

    /// Runs this trace with a raw C callback boundary.
    ///
    /// # Safety
    ///
    /// `context` must satisfy the callback's contract and both values must
    /// remain valid for the complete synchronous invocation.
    pub unsafe fn run(&self, context: *mut std::ffi::c_void, callback: TraceCallback) -> i64 {
        // SAFETY: Guaranteed by the caller contract above.
        unsafe { (self.function)(context, callback) }
    }

    pub fn run_with<F>(&self, callback: &mut F) -> i64
    where
        F: FnMut(u64) -> i64,
    {
        unsafe extern "C" fn invoke<F>(context: *mut std::ffi::c_void, pc: u64) -> i64
        where
            F: FnMut(u64) -> i64,
        {
            // SAFETY: `run_with` supplies the address of its live, uniquely
            // borrowed callback for this synchronous native invocation.
            unsafe { (&mut *context.cast::<F>())(pc) }
        }

        // SAFETY: The callback pointer remains live and uniquely borrowed until
        // the native trace returns. `invoke` never stores the pointer.
        unsafe { (self.function)((callback as *mut F).cast::<std::ffi::c_void>(), invoke::<F>) }
    }

    #[must_use]
    pub fn instructions(&self) -> usize {
        self.instructions
    }
}

impl TraceObject {
    pub fn compile(name: &str, instructions: usize) -> Result<Vec<u8>, String> {
        check_export_name(name)?;
        check_trace_limits(instructions)?;
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "true")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = ObjectBuilder::new(isa, "nivren_trace_aot", default_libcall_names())
            .map_err(|error| error.to_string())?;
        let mut module = ObjectModule::new(builder);
        let signature = trace_signature(&mut module);
        let function_id = module
            .declare_function(name, Linkage::Export, &signature)
            .map_err(|error| error.to_string())?;
        let mut context = module.make_context();
        context.func.signature = signature;
        context.func.name = UserFuncName::user(0, 1);
        define_trace_body(&mut context.func, instructions)?;
        module
            .define_function(function_id, &mut context)
            .map_err(|error| error.to_string())?;
        module.clear_context(&mut context);
        module.finish().emit().map_err(|error| error.to_string())
    }
}

fn check_trace_limits(instructions: usize) -> Result<(), String> {
    if instructions == 0 || instructions > 1_000_000 {
        Err("native traces require 1 through 1000000 instructions".into())
    } else {
        Ok(())
    }
}

fn check_export_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Err("AOT export name must contain only ASCII letters, digits, or underscore".into())
    } else {
        Ok(())
    }
}

fn trace_signature<M: Module>(module: &mut M) -> cranelift_codegen::ir::Signature {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    signature
}

fn define_trace_body(
    function: &mut cranelift_codegen::ir::Function,
    instructions: usize,
) -> Result<(), String> {
    let pointer = function.signature.params[0].value_type;
    let call_conv = function.signature.call_conv;
    let mut builder_context = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(function, &mut builder_context);
    let entry = builder.create_block();
    let dispatch = builder.create_block();
    let invalid = builder.create_block();
    let exit = builder.create_block();
    let instruction_blocks = (0..instructions)
        .map(|_| builder.create_block())
        .collect::<Vec<_>>();
    builder.append_block_params_for_function_params(entry);
    builder.append_block_param(dispatch, types::I64);
    builder.append_block_param(exit, types::I64);
    let context_var = Variable::from_u32(0);
    let callback_var = Variable::from_u32(1);
    builder.declare_var(context_var, pointer);
    builder.declare_var(callback_var, pointer);
    builder.switch_to_block(entry);
    builder.seal_block(entry);
    builder.def_var(context_var, builder.block_params(entry)[0]);
    builder.def_var(callback_var, builder.block_params(entry)[1]);
    let zero = builder.ins().iconst(types::I64, 0);
    let zero_arguments = [zero.into()];
    builder.ins().jump(dispatch, &zero_arguments);

    builder.switch_to_block(dispatch);
    let next = builder.block_params(dispatch)[0];
    let mut switch = Switch::new();
    for (index, block) in instruction_blocks.iter().enumerate() {
        switch.set_entry(
            u128::try_from(index).map_err(|_| "trace index overflow")?,
            *block,
        );
    }
    switch.emit(&mut builder, next, invalid);

    let mut callback_signature = cranelift_codegen::ir::Signature::new(call_conv);
    callback_signature.params.push(AbiParam::new(pointer));
    callback_signature.params.push(AbiParam::new(types::I64));
    callback_signature.returns.push(AbiParam::new(types::I64));
    let callback_signature = builder.import_signature(callback_signature);
    for (index, block) in instruction_blocks.into_iter().enumerate() {
        builder.switch_to_block(block);
        let context = builder.use_var(context_var);
        let callback = builder.use_var(callback_var);
        let index = builder.ins().iconst(
            types::I64,
            i64::try_from(index).map_err(|_| "trace index overflow")?,
        );
        let call = builder
            .ins()
            .call_indirect(callback_signature, callback, &[context, index]);
        let result = builder.inst_results(call)[0];
        let terminated = builder.ins().icmp_imm(
            cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
            result,
            0,
        );
        let result_arguments = [result.into()];
        builder.ins().brif(
            terminated,
            exit,
            &result_arguments,
            dispatch,
            &result_arguments,
        );
    }

    builder.switch_to_block(invalid);
    let invalid_status = builder.ins().iconst(types::I64, -3);
    builder.ins().return_(&[invalid_status]);
    builder.switch_to_block(exit);
    let exit_status = builder.block_params(exit)[0];
    builder.ins().return_(&[exit_status]);
    builder.seal_all_blocks();
    builder.finalize();
    Ok(())
}

impl AotObject {
    pub fn compile(
        name: &str,
        parameters: usize,
        slots: usize,
        operations: &[IntOp],
    ) -> Result<Vec<u8>, String> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(
                "AOT export name must contain only ASCII letters, digits, or underscore".into(),
            );
        }
        if parameters > slots || slots > 4096 || operations.len() > 1_000_000 {
            return Err("invalid AOT function limits".into());
        }
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "true")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = ObjectBuilder::new(isa, "nivren_aot", default_libcall_names())
            .map_err(|error| error.to_string())?;
        let mut module = ObjectModule::new(builder);
        define_integer_function(
            &mut module,
            name,
            Linkage::Export,
            parameters,
            slots,
            operations,
        )?;
        module.finish().emit().map_err(|error| error.to_string())
    }
}

// SAFETY: The finalized module is never mutated after construction, its code and data
// allocations remain owned by this value, and Nivren stores each compiled function behind
// a Mutex. Moving that exclusive owner to another thread does not invalidate executable
// addresses or create concurrent access to JITModule's non-Sync internals.
unsafe impl Send for CompiledFunction {}

// SAFETY: Compilation is complete before a `CompiledFunction` is published. Calls only
// execute immutable machine code and use caller-owned argument and overflow buffers; they
// never access or mutate the retained `JITModule`. The module remains owned until all calls
// have finished because callers hold a shared reference to this value.
unsafe impl Sync for CompiledFunction {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Arity,
    Overflow,
}

impl CompiledFunction {
    pub fn compile(parameters: usize, slots: usize, operations: &[IntOp]) -> Result<Self, String> {
        if parameters > slots || slots > 4096 || operations.len() > 1_000_000 {
            return Err("invalid JIT function limits".into());
        }
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "false")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = JITBuilder::with_isa(isa, default_libcall_names());
        let mut module = JITModule::new(builder);
        let pointer = module.target_config().pointer_type();
        let mut signature = module.make_signature();
        signature.params.push(AbiParam::new(pointer));
        signature.params.push(AbiParam::new(pointer));
        signature.returns.push(AbiParam::new(types::I64));
        let function_id = module
            .declare_function("nivren_hot", Linkage::Local, &signature)
            .map_err(|error| error.to_string())?;
        let mut context = module.make_context();
        context.func.signature = signature;
        context.func.name = UserFuncName::user(0, 0);
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let entry = function.create_block();
            let overflow = function.create_block();
            function.append_block_params_for_function_params(entry);
            function.switch_to_block(entry);
            function.seal_block(entry);
            let argument_pointer = function.block_params(entry)[0];
            let overflow_pointer = function.block_params(entry)[1];
            for slot in 0..slots {
                let variable = Variable::from_u32(u32::try_from(slot).unwrap());
                function.declare_var(variable, types::I64);
                let initial = if slot < parameters {
                    function.ins().load(
                        types::I64,
                        MemFlags::trusted(),
                        argument_pointer,
                        i32::try_from(slot * 8).map_err(|_| "JIT slot offset overflow")?,
                    )
                } else {
                    function.ins().iconst(types::I64, 0)
                };
                function.def_var(variable, initial);
            }
            let mut stack = Vec::new();
            let mut returned = false;
            for operation in operations {
                match operation {
                    IntOp::Constant(value) => {
                        stack.push(function.ins().iconst(types::I64, *value));
                    }
                    IntOp::Load(slot) => stack.push(function.use_var(Variable::from_u32(*slot))),
                    IntOp::Define(slot) | IntOp::Store(slot) => {
                        let value = *stack.last().ok_or("JIT stack underflow")?;
                        function.def_var(Variable::from_u32(*slot), value);
                    }
                    IntOp::Pop => {
                        stack.pop().ok_or("JIT stack underflow")?;
                    }
                    IntOp::Add | IntOp::Subtract | IntOp::Multiply => {
                        let right = stack.pop().ok_or("JIT stack underflow")?;
                        let left = stack.pop().ok_or("JIT stack underflow")?;
                        let result = match operation {
                            IntOp::Add => function.ins().iadd(left, right),
                            IntOp::Subtract => function.ins().isub(left, right),
                            IntOp::Multiply => function.ins().imul(left, right),
                            _ => unreachable!(),
                        };
                        let overflowed = match operation {
                            IntOp::Add => {
                                let first = function.ins().bxor(left, result);
                                let second = function.ins().bxor(right, result);
                                let both = function.ins().band(first, second);
                                function.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                    both,
                                    0,
                                )
                            }
                            IntOp::Subtract => {
                                let first = function.ins().bxor(left, right);
                                let second = function.ins().bxor(left, result);
                                let both = function.ins().band(first, second);
                                function.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                    both,
                                    0,
                                )
                            }
                            IntOp::Multiply => {
                                let high = function.ins().smulhi(left, right);
                                let sign = function.ins().sshr_imm(result, 63);
                                function.ins().icmp(
                                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                                    high,
                                    sign,
                                )
                            }
                            _ => unreachable!(),
                        };
                        branch_if_overflow(&mut function, overflowed, overflow);
                        stack.push(result);
                    }
                    IntOp::Negate => {
                        let value = stack.pop().ok_or("JIT stack underflow")?;
                        let overflowed = function.ins().icmp_imm(
                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                            value,
                            i64::MIN,
                        );
                        branch_if_overflow(&mut function, overflowed, overflow);
                        stack.push(function.ins().ineg(value));
                    }
                    IntOp::Return => {
                        let value = stack.pop().ok_or("JIT stack underflow")?;
                        function.ins().return_(&[value]);
                        returned = true;
                        break;
                    }
                }
            }
            if !returned {
                return Err("JIT function has no give".into());
            }
            function.switch_to_block(overflow);
            function.seal_block(overflow);
            let one = function.ins().iconst(types::I8, 1);
            function
                .ins()
                .store(MemFlags::trusted(), one, overflow_pointer, 0);
            let zero = function.ins().iconst(types::I64, 0);
            function.ins().return_(&[zero]);
            function.finalize();
        }
        module
            .define_function(function_id, &mut context)
            .map_err(|error| error.to_string())?;
        module.clear_context(&mut context);
        module
            .finalize_definitions()
            .map_err(|error| error.to_string())?;
        let pointer = module.get_finalized_function(function_id);
        let function = unsafe {
            std::mem::transmute::<*const u8, unsafe extern "C" fn(*const i64, *mut u8) -> i64>(
                pointer,
            )
        };
        Ok(Self {
            _module: module,
            function,
            parameters,
        })
    }

    pub fn call(&self, arguments: &[i64]) -> Result<i64, CallError> {
        if arguments.len() != self.parameters {
            return Err(CallError::Arity);
        }
        let mut overflow = 0u8;
        let value = unsafe { (self.function)(arguments.as_ptr(), &mut overflow) };
        if overflow == 0 {
            Ok(value)
        } else {
            Err(CallError::Overflow)
        }
    }
}

fn branch_if_overflow(
    function: &mut FunctionBuilder<'_>,
    condition: cranelift_codegen::ir::Value,
    overflow: cranelift_codegen::ir::Block,
) {
    let next = function.create_block();
    function.ins().brif(condition, overflow, &[], next, &[]);
    function.switch_to_block(next);
    function.seal_block(next);
}

fn define_integer_function<M: Module>(
    module: &mut M,
    name: &str,
    linkage: Linkage,
    parameters: usize,
    slots: usize,
    operations: &[IntOp],
) -> Result<(), String> {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let function_id = module
        .declare_function(name, linkage, &signature)
        .map_err(|error| error.to_string())?;
    let mut context = module.make_context();
    context.func.signature = signature;
    context.func.name = UserFuncName::user(0, 0);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        let overflow = function.create_block();
        function.append_block_params_for_function_params(entry);
        function.switch_to_block(entry);
        function.seal_block(entry);
        let argument_pointer = function.block_params(entry)[0];
        let overflow_pointer = function.block_params(entry)[1];
        for slot in 0..slots {
            let variable = Variable::from_u32(u32::try_from(slot).unwrap());
            function.declare_var(variable, types::I64);
            let initial = if slot < parameters {
                function.ins().load(
                    types::I64,
                    MemFlags::trusted(),
                    argument_pointer,
                    i32::try_from(slot * 8).map_err(|_| "AOT slot offset overflow")?,
                )
            } else {
                function.ins().iconst(types::I64, 0)
            };
            function.def_var(variable, initial);
        }
        let mut stack = Vec::new();
        let mut returned = false;
        for operation in operations {
            match operation {
                IntOp::Constant(value) => stack.push(function.ins().iconst(types::I64, *value)),
                IntOp::Load(slot) => stack.push(function.use_var(Variable::from_u32(*slot))),
                IntOp::Define(slot) | IntOp::Store(slot) => {
                    let value = *stack.last().ok_or("AOT stack underflow")?;
                    function.def_var(Variable::from_u32(*slot), value);
                }
                IntOp::Pop => {
                    stack.pop().ok_or("AOT stack underflow")?;
                }
                IntOp::Add | IntOp::Subtract | IntOp::Multiply => {
                    let right = stack.pop().ok_or("AOT stack underflow")?;
                    let left = stack.pop().ok_or("AOT stack underflow")?;
                    let result = match operation {
                        IntOp::Add => function.ins().iadd(left, right),
                        IntOp::Subtract => function.ins().isub(left, right),
                        IntOp::Multiply => function.ins().imul(left, right),
                        _ => unreachable!(),
                    };
                    let overflowed = match operation {
                        IntOp::Add => {
                            let first = function.ins().bxor(left, result);
                            let second = function.ins().bxor(right, result);
                            let both = function.ins().band(first, second);
                            function.ins().icmp_imm(
                                cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                both,
                                0,
                            )
                        }
                        IntOp::Subtract => {
                            let first = function.ins().bxor(left, right);
                            let second = function.ins().bxor(left, result);
                            let both = function.ins().band(first, second);
                            function.ins().icmp_imm(
                                cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                both,
                                0,
                            )
                        }
                        IntOp::Multiply => {
                            let high = function.ins().smulhi(left, right);
                            let sign = function.ins().sshr_imm(result, 63);
                            function.ins().icmp(
                                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                                high,
                                sign,
                            )
                        }
                        _ => unreachable!(),
                    };
                    branch_if_overflow(&mut function, overflowed, overflow);
                    stack.push(result);
                }
                IntOp::Negate => {
                    let value = stack.pop().ok_or("AOT stack underflow")?;
                    let overflowed = function.ins().icmp_imm(
                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                        value,
                        i64::MIN,
                    );
                    branch_if_overflow(&mut function, overflowed, overflow);
                    stack.push(function.ins().ineg(value));
                }
                IntOp::Return => {
                    let value = stack.pop().ok_or("AOT stack underflow")?;
                    function.ins().return_(&[value]);
                    returned = true;
                    break;
                }
            }
        }
        if !returned {
            return Err("AOT function has no give".into());
        }
        function.switch_to_block(overflow);
        function.seal_block(overflow);
        let one = function.ins().iconst(types::I8, 1);
        function
            .ins()
            .store(MemFlags::trusted(), one, overflow_pointer, 0);
        let zero = function.ins().iconst(types::I64, 0);
        function.ins().return_(&[zero]);
        function.finalize();
    }
    module
        .define_function(function_id, &mut context)
        .map_err(|error| error.to_string())?;
    module.clear_context(&mut context);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{AotObject, CallError, CompiledFunction, CompiledTrace, IntOp, TraceObject};

    struct TraceState {
        visited: Vec<u64>,
        instructions: u64,
    }

    unsafe extern "C" fn trace_step(context: *mut std::ffi::c_void, pc: u64) -> i64 {
        let state = unsafe { &mut *context.cast::<TraceState>() };
        state.visited.push(pc);
        if pc + 1 == state.instructions {
            -1
        } else {
            i64::try_from(pc + 1).unwrap()
        }
    }

    #[test]
    fn native_integer_code_executes_and_preserves_overflow() {
        let function = CompiledFunction::compile(
            2,
            2,
            &[IntOp::Load(0), IntOp::Load(1), IntOp::Add, IntOp::Return],
        )
        .unwrap();
        assert_eq!(function.call(&[20, 22]), Ok(42));
        assert_eq!(function.call(&[i64::MAX, 1]), Err(CallError::Overflow));
    }

    #[test]
    fn native_aot_objects_are_deterministic_and_nonempty() {
        let operations = [
            IntOp::Load(0),
            IntOp::Constant(2),
            IntOp::Multiply,
            IntOp::Return,
        ];
        let first = AotObject::compile("nivren_double", 1, 1, &operations).unwrap();
        let second = AotObject::compile("nivren_double", 1, 1, &operations).unwrap();
        assert!(first.len() > 64);
        assert_eq!(first, second);
        assert!(AotObject::compile("invalid-name", 1, 1, &operations).is_err());
    }

    #[test]
    fn finalized_native_functions_are_safe_to_call_concurrently() {
        let function = Arc::new(
            CompiledFunction::compile(
                2,
                2,
                &[IntOp::Load(0), IntOp::Load(1), IntOp::Add, IntOp::Return],
            )
            .unwrap(),
        );
        let workers = (0..8)
            .map(|worker| {
                let function = function.clone();
                std::thread::spawn(move || {
                    for value in 0..1_000 {
                        assert_eq!(function.call(&[worker, value]), Ok(worker + value));
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn complete_program_trace_executes_native_control_and_is_reproducible() {
        let trace = CompiledTrace::compile(4).unwrap();
        let mut state = TraceState {
            visited: vec![],
            instructions: 4,
        };
        // SAFETY: State and callback remain live for the synchronous trace.
        assert_eq!(
            unsafe { trace.run((&mut state as *mut TraceState).cast(), trace_step) },
            -1
        );
        assert_eq!(state.visited, vec![0, 1, 2, 3]);
        assert_eq!(trace.instructions(), 4);

        let first = TraceObject::compile("nivren_program", 4).unwrap();
        let second = TraceObject::compile("nivren_program", 4).unwrap();
        assert!(first.len() > 64);
        assert_eq!(first, second);
        assert!(TraceObject::compile("bad-name", 4).is_err());
        assert!(CompiledTrace::compile(0).is_err());
    }
}
