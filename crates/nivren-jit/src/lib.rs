use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, UserFuncName, types};
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};

/// Cranelift now allocates variable numbers itself, so slots are mapped through
/// the order in which they were declared rather than assumed to match.
fn slot_variable(variables: &[Variable], slot: u32) -> Result<Variable, String> {
    variables
        .get(usize::try_from(slot).map_err(|_| "JIT slot index overflow")?)
        .copied()
        .ok_or_else(|| format!("JIT slot {slot} was never declared"))
}
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
    /// Checked signed division; a zero divisor faults with code 2 and
    /// `i64::MIN / -1` faults with code 1.
    Divide,
    /// Checked signed remainder; a zero divisor faults with code 3 and
    /// `i64::MIN % -1` faults with code 1.
    Modulo,
    Negate,
    /// Boolean not over a 0/1 value.
    Not,
    /// Pops two integers and pushes the 0/1 comparison result.
    Compare(IntCondition),
    /// Unconditional branch to the operation index.
    Jump(u32),
    /// Peeks the 0/1 value on top of the stack (leaving it in place, like
    /// the bytecode VM) and branches to the operation index when it is 0.
    JumpIfFalse(u32),
    /// Placeholder keeping operation indices aligned with source bytecode.
    Nop,
    /// Calls another planned function in the same compiled program: pops
    /// `arity` arguments (pushed left to right) and pushes the integer
    /// result. Faults propagate to the caller.
    CallPlanned {
        function: u32,
        arity: u32,
    },
    /// The loop-seed null value: a machine zero whose kind analysis forbids
    /// every use except `Pop` (and joins that stay unobserved).
    NullConstant,
    Return,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntCondition {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

pub struct CompiledFunction {
    module: Option<JITModule>,
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
    module: Option<JITModule>,
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
            module: Some(module),
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

impl Drop for CompiledTrace {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            // SAFETY: The module and its only exposed function pointer are
            // owned by this value. Drop runs only after the last borrow (and
            // last Arc owner in the runtime) has ended, so no trace can still
            // be executing or call the pointer after this point.
            unsafe { module.free_memory() };
        }
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
    let context_var = builder.declare_var(pointer);
    let callback_var = builder.declare_var(pointer);
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
            false,
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
    DivisionByZero,
    RemainderByZero,
    CallDepth,
}

fn fault_error(code: u8) -> CallError {
    match code {
        2 => CallError::DivisionByZero,
        3 => CallError::RemainderByZero,
        4 => CallError::CallDepth,
        _ => CallError::Overflow,
    }
}

impl CompiledFunction {
    pub fn compile(parameters: usize, slots: usize, operations: &[IntOp]) -> Result<Self, String> {
        Self::compile_with(parameters, slots, operations, false)
    }

    /// Compiles with slot write-back: every `Return` first stores the final
    /// value of every slot into the argument buffer, so a caller-owned
    /// buffer of `slots` integers receives the chunk's top-level bindings.
    pub fn compile_root(slots: usize, operations: &[IntOp]) -> Result<Self, String> {
        Self::compile_with(slots, slots, operations, true)
    }

    fn compile_with(
        parameters: usize,
        slots: usize,
        operations: &[IntOp],
        writeback_slots: bool,
    ) -> Result<Self, String> {
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
        let function_id = define_integer_function(
            &mut module,
            "nivren_hot",
            Linkage::Local,
            parameters,
            slots,
            operations,
            writeback_slots,
        )?;
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
            module: Some(module),
            function,
            parameters,
        })
    }

    #[inline]
    pub fn call(&self, arguments: &[i64]) -> Result<i64, CallError> {
        if arguments.len() != self.parameters {
            return Err(CallError::Arity);
        }
        let mut fault = 0u8;
        let value = unsafe { (self.function)(arguments.as_ptr(), &mut fault) };
        if fault == 0 {
            Ok(value)
        } else {
            Err(fault_error(fault))
        }
    }

    /// Calls a root compilation, reading initial slot values from `slots`
    /// and writing every slot's final value back into it on success.
    #[inline]
    pub fn call_with_slots(&self, slots: &mut [i64]) -> Result<i64, CallError> {
        if slots.len() != self.parameters {
            return Err(CallError::Arity);
        }
        let mut fault = 0u8;
        let value = unsafe { (self.function)(slots.as_ptr(), &mut fault) };
        if fault == 0 {
            Ok(value)
        } else {
            Err(fault_error(fault))
        }
    }
}

impl Drop for CompiledFunction {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            // SAFETY: The finalized pointer cannot outlive this wrapper and
            // Drop cannot run while a safe call still borrows the wrapper.
            unsafe { module.free_memory() };
        }
    }
}

fn branch_if_fault(
    function: &mut FunctionBuilder<'_>,
    condition: cranelift_codegen::ir::Value,
    fault: cranelift_codegen::ir::Block,
    code: i64,
) {
    let next = function.create_block();
    let code = function.ins().iconst(types::I64, code);
    function
        .ins()
        .brif(condition, fault, &[code.into()], next, &[]);
    function.switch_to_block(next);
    function.seal_block(next);
}

/// Stack effect of one operation: values popped and pushed. `Define` and
/// `Store` peek without popping, mirroring the bytecode VM.
fn int_op_effect(operation: &IntOp) -> (usize, usize) {
    match operation {
        IntOp::Constant(_) | IntOp::NullConstant | IntOp::Load(_) => (0, 1),
        IntOp::Define(_) | IntOp::Store(_) | IntOp::Nop | IntOp::Jump(_) => (0, 0),
        IntOp::Pop | IntOp::Return => (1, 0),
        IntOp::JumpIfFalse(_) => (0, 0),
        IntOp::Add
        | IntOp::Subtract
        | IntOp::Multiply
        | IntOp::Divide
        | IntOp::Modulo
        | IntOp::Compare(_) => (2, 1),
        IntOp::Negate | IntOp::Not => (1, 1),
        IntOp::CallPlanned { arity, .. } => (*arity as usize, 1),
    }
}

/// Computes the value-stack depth entering every operation, or fails when
/// control-flow paths disagree, jump targets fall outside the program, or a
/// stack underflows. Unreachable operations report `None`.
fn int_stack_depths(operations: &[IntOp]) -> Result<Vec<Option<usize>>, String> {
    let mut depths: Vec<Option<usize>> = vec![None; operations.len()];
    let mut worklist = vec![(0usize, 0usize)];
    while let Some((index, depth)) = worklist.pop() {
        if index >= operations.len() {
            return Err("JIT control flow escapes the program".into());
        }
        match depths[index] {
            Some(existing) if existing == depth => continue,
            Some(_) => return Err("JIT stack depths disagree at a join".into()),
            None => depths[index] = Some(depth),
        }
        let operation = &operations[index];
        let (pops, pushes) = int_op_effect(operation);
        if depth < pops {
            return Err("JIT stack underflow".into());
        }
        let next_depth = depth - pops + pushes;
        match operation {
            IntOp::Jump(target) => worklist.push((*target as usize, next_depth)),
            IntOp::JumpIfFalse(target) => {
                worklist.push((*target as usize, next_depth));
                worklist.push((index + 1, next_depth));
            }
            IntOp::Return => {}
            _ => worklist.push((index + 1, next_depth)),
        }
    }
    Ok(depths)
}

/// A function inside a [`CompiledProgram`]: internal register-argument ABI
/// with a call-depth parameter and a shared fault pointer.
pub struct PlanFunction {
    pub parameters: usize,
    pub slots: usize,
    pub operations: Vec<IntOp>,
}

/// The top-level chunk of a [`CompiledProgram`]: memory-argument ABI with
/// slot write-back, entered at call depth zero.
pub struct PlanRoot {
    pub slots: usize,
    pub operations: Vec<IntOp>,
}

/// A whole planned program compiled into one JIT module: the root chunk plus
/// every planned function, calling one another directly in native code.
pub struct CompiledProgram {
    module: Option<JITModule>,
    root: unsafe extern "C" fn(*const i64, *mut u8) -> i64,
    slots: usize,
}

// SAFETY: See `CompiledFunction`; a program owns its finalized module the
// same way and only exposes calls through caller-owned buffers.
unsafe impl Send for CompiledProgram {}
// SAFETY: Calls execute immutable code with caller-owned buffers.
unsafe impl Sync for CompiledProgram {}

impl CompiledProgram {
    pub fn compile(
        functions: &[PlanFunction],
        root: &PlanRoot,
        depth_limit: i64,
    ) -> Result<Self, String> {
        if functions.len() > 1024 || root.slots > 4096 {
            return Err("invalid JIT program limits".into());
        }
        for function in functions {
            if function.parameters > function.slots
                || function.slots > 4096
                || function.operations.len() > 1_000_000
            {
                return Err("invalid JIT function limits".into());
            }
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

        let mut function_ids = Vec::with_capacity(functions.len());
        for (index, function) in functions.iter().enumerate() {
            let mut signature = module.make_signature();
            for _ in 0..function.parameters {
                signature.params.push(AbiParam::new(types::I64));
            }
            signature.params.push(AbiParam::new(types::I64)); // depth
            signature.params.push(AbiParam::new(pointer)); // fault
            signature.returns.push(AbiParam::new(types::I64));
            let id = module
                .declare_function(&format!("nivren_plan_{index}"), Linkage::Local, &signature)
                .map_err(|error| error.to_string())?;
            function_ids.push(id);
        }
        let mut root_signature = module.make_signature();
        root_signature.params.push(AbiParam::new(pointer));
        root_signature.params.push(AbiParam::new(pointer));
        root_signature.returns.push(AbiParam::new(types::I64));
        let root_id = module
            .declare_function("nivren_plan_root", Linkage::Local, &root_signature)
            .map_err(|error| error.to_string())?;

        for (index, function) in functions.iter().enumerate() {
            let mut context = module.make_context();
            context.func.signature = module
                .declarations()
                .get_function_decl(function_ids[index])
                .signature
                .clone();
            context.func.name = UserFuncName::user(0, index as u32 + 1);
            emit_plan_body(
                &mut module,
                &mut context,
                &function_ids,
                EntryKind::Registers { depth_limit },
                function.parameters,
                function.slots,
                &function.operations,
            )?;
            module
                .define_function(function_ids[index], &mut context)
                .map_err(|error| error.to_string())?;
            module.clear_context(&mut context);
        }
        {
            let mut context = module.make_context();
            context.func.signature = root_signature;
            context.func.name = UserFuncName::user(0, 0);
            emit_plan_body(
                &mut module,
                &mut context,
                &function_ids,
                EntryKind::Memory { writeback: true },
                root.slots,
                root.slots,
                &root.operations,
            )?;
            module
                .define_function(root_id, &mut context)
                .map_err(|error| error.to_string())?;
            module.clear_context(&mut context);
        }
        module
            .finalize_definitions()
            .map_err(|error| error.to_string())?;
        let pointer = module.get_finalized_function(root_id);
        let root_function = unsafe {
            std::mem::transmute::<*const u8, unsafe extern "C" fn(*const i64, *mut u8) -> i64>(
                pointer,
            )
        };
        Ok(Self {
            module: Some(module),
            root: root_function,
            slots: root.slots,
        })
    }

    /// Runs the program root, reading initial slot values from `slots` and
    /// writing every slot's final value back on success.
    #[inline]
    pub fn call_root(&self, slots: &mut [i64]) -> Result<i64, CallError> {
        if slots.len() != self.slots {
            return Err(CallError::Arity);
        }
        let mut fault = 0u8;
        let value = unsafe { (self.root)(slots.as_ptr(), &mut fault) };
        match fault {
            0 => Ok(value),
            2 => Err(CallError::DivisionByZero),
            3 => Err(CallError::RemainderByZero),
            4 => Err(CallError::CallDepth),
            _ => Err(CallError::Overflow),
        }
    }
}

impl Drop for CompiledProgram {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            // SAFETY: The finalized pointer cannot outlive this wrapper and
            // Drop cannot run while a safe call still borrows the wrapper.
            unsafe { module.free_memory() };
        }
    }
}

enum EntryKind {
    /// Arguments and slots live in a caller-owned buffer; on `Return`, every
    /// slot is written back when `writeback` is set. Entered at depth zero.
    Memory { writeback: bool },
    /// Arguments arrive in registers together with the call depth, which is
    /// checked against the limit on entry.
    Registers { depth_limit: i64 },
}

#[allow(clippy::too_many_lines)]
fn emit_plan_body<M: Module>(
    module: &mut M,
    context: &mut cranelift_codegen::Context,
    callees: &[cranelift_module::FuncId],
    entry_kind: EntryKind,
    parameters: usize,
    slots: usize,
    operations: &[IntOp],
) -> Result<(), String> {
    let depths = int_stack_depths(operations)?;
    let mut callee_refs = Vec::with_capacity(callees.len());
    for id in callees {
        callee_refs.push(module.declare_func_in_func(*id, &mut context.func));
    }
    let mut builder_context = FunctionBuilderContext::new();
    let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let entry = function.create_block();
    let fault = function.create_block();
    function.append_block_params_for_function_params(entry);
    function.append_block_param(fault, types::I64);
    function.switch_to_block(entry);
    let entry_params = function.block_params(entry).to_vec();
    let (argument_pointer, fault_pointer, depth, writeback) = match &entry_kind {
        EntryKind::Memory { writeback } => {
            let zero_depth = function.ins().iconst(types::I64, 0);
            (
                Some(entry_params[0]),
                entry_params[1],
                zero_depth,
                *writeback,
            )
        }
        EntryKind::Registers { depth_limit } => {
            let depth = entry_params[parameters];
            let fault_pointer = entry_params[parameters + 1];
            let exceeded = function.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                depth,
                *depth_limit,
            );
            branch_if_fault(&mut function, exceeded, fault, 4);
            (None, fault_pointer, depth, false)
        }
    };
    let mut variables = Vec::with_capacity(slots);
    #[allow(clippy::needless_range_loop)]
    for slot in 0..slots {
        let variable = function.declare_var(types::I64);
        variables.push(variable);
        let initial = if let Some(argument_pointer) = argument_pointer {
            if slot < parameters {
                function.ins().load(
                    types::I64,
                    MemFlags::trusted(),
                    argument_pointer,
                    i32::try_from(slot * 8).map_err(|_| "JIT slot offset overflow")?,
                )
            } else {
                function.ins().iconst(types::I64, 0)
            }
        } else if slot < parameters {
            entry_params[slot]
        } else {
            function.ins().iconst(types::I64, 0)
        };
        function.def_var(variable, initial);
    }

    let mut block_starts = std::collections::BTreeSet::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            IntOp::Jump(target) => {
                block_starts.insert(*target as usize);
            }
            IntOp::JumpIfFalse(target) => {
                block_starts.insert(*target as usize);
                block_starts.insert(index + 1);
            }
            _ => {}
        }
    }
    let mut blocks = std::collections::BTreeMap::new();
    for index in &block_starts {
        if let Some(Some(depth)) = depths.get(*index).copied() {
            let block = function.create_block();
            for _ in 0..depth {
                function.append_block_param(block, types::I64);
            }
            blocks.insert(*index, block);
        }
    }
    let block_args = |stack: &[cranelift_codegen::ir::Value]| {
        stack
            .iter()
            .map(|value| cranelift_codegen::ir::BlockArg::from(*value))
            .collect::<Vec<_>>()
    };

    let mut stack: Vec<cranelift_codegen::ir::Value> = Vec::new();
    let mut terminated = false;
    let mut in_dead_code = false;
    for (index, operation) in operations.iter().enumerate() {
        if let Some(block) = blocks.get(&index) {
            if !terminated && !in_dead_code {
                function.ins().jump(*block, &block_args(&stack));
            }
            function.switch_to_block(*block);
            stack = function.block_params(*block).to_vec();
            terminated = false;
            in_dead_code = false;
        } else if terminated || depths[index].is_none() {
            in_dead_code = true;
            continue;
        }
        if in_dead_code {
            continue;
        }
        match operation {
            IntOp::Nop => {}
            IntOp::Constant(value) => {
                stack.push(function.ins().iconst(types::I64, *value));
            }
            IntOp::NullConstant => {
                stack.push(function.ins().iconst(types::I64, 0));
            }
            IntOp::Load(slot) => {
                stack.push(function.use_var(slot_variable(&variables, *slot)?));
            }
            IntOp::Define(slot) | IntOp::Store(slot) => {
                let value = *stack.last().ok_or("JIT stack underflow")?;
                function.def_var(slot_variable(&variables, *slot)?, value);
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
                branch_if_fault(&mut function, overflowed, fault, 1);
                stack.push(result);
            }
            IntOp::Divide | IntOp::Modulo => {
                let right = stack.pop().ok_or("JIT stack underflow")?;
                let left = stack.pop().ok_or("JIT stack underflow")?;
                let zero_code = if matches!(operation, IntOp::Divide) {
                    2
                } else {
                    3
                };
                let zero = function.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    right,
                    0,
                );
                branch_if_fault(&mut function, zero, fault, zero_code);
                let minimum = function.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    left,
                    i64::MIN,
                );
                let negative_one = function.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    right,
                    -1,
                );
                let overflowed = function.ins().band(minimum, negative_one);
                branch_if_fault(&mut function, overflowed, fault, 1);
                let result = if matches!(operation, IntOp::Divide) {
                    function.ins().sdiv(left, right)
                } else {
                    function.ins().srem(left, right)
                };
                stack.push(result);
            }
            IntOp::Negate => {
                let value = stack.pop().ok_or("JIT stack underflow")?;
                let overflowed = function.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    value,
                    i64::MIN,
                );
                branch_if_fault(&mut function, overflowed, fault, 1);
                stack.push(function.ins().ineg(value));
            }
            IntOp::Not => {
                let value = stack.pop().ok_or("JIT stack underflow")?;
                stack.push(function.ins().bxor_imm(value, 1));
            }
            IntOp::Compare(condition) => {
                let right = stack.pop().ok_or("JIT stack underflow")?;
                let left = stack.pop().ok_or("JIT stack underflow")?;
                let code = match condition {
                    IntCondition::Equal => cranelift_codegen::ir::condcodes::IntCC::Equal,
                    IntCondition::NotEqual => cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                    IntCondition::Less => cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                    IntCondition::LessEqual => {
                        cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual
                    }
                    IntCondition::Greater => {
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan
                    }
                    IntCondition::GreaterEqual => {
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual
                    }
                };
                let flag = function.ins().icmp(code, left, right);
                stack.push(function.ins().uextend(types::I64, flag));
            }
            IntOp::CallPlanned {
                function: callee,
                arity,
            } => {
                let arity = *arity as usize;
                let callee_ref = callee_refs
                    .get(*callee as usize)
                    .ok_or("JIT call references an unknown planned function")?;
                if stack.len() < arity {
                    return Err("JIT stack underflow".into());
                }
                let mut arguments = stack.split_off(stack.len() - arity);
                let next_depth = function.ins().iadd_imm(depth, 1);
                arguments.push(next_depth);
                arguments.push(fault_pointer);
                let call = function.ins().call(*callee_ref, &arguments);
                let result = function.inst_results(call)[0];
                let raised = function
                    .ins()
                    .load(types::I8, MemFlags::trusted(), fault_pointer, 0);
                let raised = function.ins().icmp_imm(
                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                    raised,
                    0,
                );
                let next = function.create_block();
                let propagate = function.create_block();
                function.ins().brif(raised, propagate, &[], next, &[]);
                function.switch_to_block(propagate);
                function.seal_block(propagate);
                let zero = function.ins().iconst(types::I64, 0);
                function.ins().return_(&[zero]);
                function.switch_to_block(next);
                function.seal_block(next);
                stack.push(result);
            }
            IntOp::Jump(target) => {
                let block = blocks
                    .get(&(*target as usize))
                    .ok_or("JIT jump target is unreachable")?;
                function.ins().jump(*block, &block_args(&stack));
                terminated = true;
            }
            IntOp::JumpIfFalse(target) => {
                let condition = *stack.last().ok_or("JIT stack underflow")?;
                let target_block = blocks
                    .get(&(*target as usize))
                    .ok_or("JIT jump target is unreachable")?;
                let next_block = blocks
                    .get(&(index + 1))
                    .ok_or("JIT branch fall-through is unreachable")?;
                let arguments = block_args(&stack);
                function.ins().brif(
                    condition,
                    *next_block,
                    &arguments,
                    *target_block,
                    &arguments,
                );
                terminated = true;
            }
            IntOp::Return => {
                let value = stack.pop().ok_or("JIT stack underflow")?;
                if writeback {
                    if let Some(argument_pointer) = argument_pointer {
                        for (slot, variable) in variables.iter().enumerate() {
                            let current = function.use_var(*variable);
                            function.ins().store(
                                MemFlags::trusted(),
                                current,
                                argument_pointer,
                                i32::try_from(slot * 8).map_err(|_| "JIT slot offset overflow")?,
                            );
                        }
                    }
                }
                function.ins().return_(&[value]);
                terminated = true;
            }
        }
    }
    if !terminated && !in_dead_code {
        return Err("JIT function has no give".into());
    }
    function.switch_to_block(fault);
    let code = function.block_params(fault)[0];
    let code = function.ins().ireduce(types::I8, code);
    function
        .ins()
        .store(MemFlags::trusted(), code, fault_pointer, 0);
    let zero = function.ins().iconst(types::I64, 0);
    function.ins().return_(&[zero]);
    function.seal_all_blocks();
    function.finalize();
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn define_integer_function<M: Module>(
    module: &mut M,
    name: &str,
    linkage: Linkage,
    parameters: usize,
    slots: usize,
    operations: &[IntOp],
    writeback_slots: bool,
) -> Result<cranelift_module::FuncId, String> {
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
    emit_plan_body(
        module,
        &mut context,
        &[],
        EntryKind::Memory {
            writeback: writeback_slots,
        },
        parameters,
        slots,
        operations,
    )?;
    module
        .define_function(function_id, &mut context)
        .map_err(|error| error.to_string())?;
    module.clear_context(&mut context);
    Ok(function_id)
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
    fn compiled_programs_call_planned_functions_natively() {
        use super::{CompiledProgram, IntCondition, PlanFunction, PlanRoot};
        // fibonacci(value): if value < 2 return value;
        // return fibonacci(value-1) + fibonacci(value-2)
        let fibonacci = PlanFunction {
            parameters: 1,
            slots: 1,
            operations: {
                let mut ops = vec![
                    IntOp::Load(0),
                    IntOp::Constant(2),
                    IntOp::Compare(IntCondition::Less),
                    IntOp::JumpIfFalse(8),
                    IntOp::Pop,
                    IntOp::Load(0),
                    IntOp::Return,
                    IntOp::Nop,
                    IntOp::Pop, // 8
                    IntOp::Load(0),
                    IntOp::Constant(1),
                    IntOp::Subtract,
                    IntOp::CallPlanned {
                        function: 0,
                        arity: 1,
                    },
                    IntOp::Load(0),
                    IntOp::Constant(2),
                    IntOp::Subtract,
                    IntOp::CallPlanned {
                        function: 0,
                        arity: 1,
                    },
                    IntOp::Add,
                    IntOp::Return,
                ];
                ops.truncate(ops.len());
                ops
            },
        };
        let root = PlanRoot {
            slots: 0,
            operations: vec![
                IntOp::Constant(20),
                IntOp::CallPlanned {
                    function: 0,
                    arity: 1,
                },
                IntOp::Return,
            ],
        };
        let program = CompiledProgram::compile(&[fibonacci], &root, 256).unwrap();
        assert_eq!(program.call_root(&mut []).unwrap(), 6765);

        // Unbounded recursion trips the depth guard instead of the stack.
        let forever = PlanFunction {
            parameters: 1,
            slots: 1,
            operations: vec![
                IntOp::Load(0),
                IntOp::CallPlanned {
                    function: 0,
                    arity: 1,
                },
                IntOp::Return,
            ],
        };
        let root = PlanRoot {
            slots: 0,
            operations: vec![
                IntOp::Constant(1),
                IntOp::CallPlanned {
                    function: 0,
                    arity: 1,
                },
                IntOp::Return,
            ],
        };
        let program = CompiledProgram::compile(&[forever], &root, 256).unwrap();
        assert_eq!(program.call_root(&mut []), Err(CallError::CallDepth));
    }

    #[test]
    fn native_loops_compare_divide_and_report_faults() {
        use super::IntCondition;
        // total = 0; index = 0; while index < limit { total += index; index += 1 } return total / 2
        // total = 0; index = 0; while index < limit { total += index;
        // index += 1 } return total / 2 — with the VM's branch shape: the
        // branch peeks its condition, so both continuations pop it.
        let ops = vec![
            IntOp::Constant(0),
            IntOp::Define(1), // total
            IntOp::Pop,
            IntOp::Constant(0),
            IntOp::Define(2), // index
            IntOp::Pop,
            // 6: loop head
            IntOp::Load(2),
            IntOp::Load(0), // limit parameter
            IntOp::Compare(IntCondition::Less),
            IntOp::JumpIfFalse(21),
            IntOp::Pop, // 10: drop the condition on the loop path
            IntOp::Load(1),
            IntOp::Load(2),
            IntOp::Add,
            IntOp::Store(1),
            IntOp::Pop,
            IntOp::Load(2),
            IntOp::Constant(1),
            IntOp::Add,
            IntOp::Store(2),
            IntOp::Pop, // 20
            IntOp::Jump(6),
        ];
        let mut ops = ops;
        ops[21] = IntOp::Jump(6);
        ops.push(IntOp::Pop); // 22: drop the condition on the exit path
        ops.push(IntOp::Load(1));
        ops.push(IntOp::Constant(2));
        ops.push(IntOp::Divide);
        ops.push(IntOp::Return);
        ops[9] = IntOp::JumpIfFalse(22);
        let compiled = CompiledFunction::compile(1, 3, &ops).unwrap();
        // sum 0..10 = 45; 45 / 2 = 22
        assert_eq!(compiled.call(&[10]).unwrap(), 22);
        assert_eq!(compiled.call(&[0]).unwrap(), 0);

        let divide_by_zero = vec![
            IntOp::Load(0),
            IntOp::Constant(0),
            IntOp::Divide,
            IntOp::Return,
        ];
        let compiled = CompiledFunction::compile(1, 1, &divide_by_zero).unwrap();
        assert_eq!(compiled.call(&[7]), Err(CallError::DivisionByZero));

        let remainder_by_zero = vec![
            IntOp::Load(0),
            IntOp::Constant(0),
            IntOp::Modulo,
            IntOp::Return,
        ];
        let compiled = CompiledFunction::compile(1, 1, &remainder_by_zero).unwrap();
        assert_eq!(compiled.call(&[7]), Err(CallError::RemainderByZero));

        let writeback = vec![
            IntOp::Constant(41),
            IntOp::Constant(1),
            IntOp::Add,
            IntOp::Store(0),
            IntOp::Return,
        ];
        let compiled = CompiledFunction::compile_root(1, &writeback).unwrap();
        let mut slots = [0i64];
        assert_eq!(compiled.call_with_slots(&mut slots).unwrap(), 42);
        assert_eq!(slots[0], 42);
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
