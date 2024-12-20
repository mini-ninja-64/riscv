#![deny(warnings)]

extern crate core;
extern crate proc_macro;
extern crate proc_macro2;
extern crate quote;
extern crate syn;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    parse::{self, Parse},
    parse_macro_input, parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    FnArg, ItemFn, LitInt, LitStr, PatType, Path, ReturnType, Token, Type, Visibility,
};

use proc_macro::TokenStream;

/// Attribute to declare the entry point of the program
///
/// **IMPORTANT**: This attribute must appear exactly *once* in the dependency graph. Also, if you
/// are using Rust 1.30 the attribute must be used on a reachable item (i.e. there must be no
/// private modules between the item and the root of the crate); if the item is in the root of the
/// crate you'll be fine. This reachability restriction doesn't apply to Rust 1.31 and newer releases.
///
/// The specified function will be called by the reset handler *after* RAM has been initialized.
/// If present, the FPU will also be enabled before the function is called.
///
/// The type of the specified function must be `[unsafe] fn() -> !` (never ending function)
///
/// # Properties
///
/// The entry point will be called by the reset handler. The program can't reference to the entry
/// point, much less invoke it.
///
/// # Examples
///
/// - Simple entry point
///
/// ``` no_run
/// # #![no_main]
/// # use riscv_rt_macros::entry;
/// #[entry]
/// fn main() -> ! {
///     loop {
///         /* .. */
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn entry(args: TokenStream, input: TokenStream) -> TokenStream {
    let f = parse_macro_input!(input as ItemFn);

    #[cfg(not(feature = "u-boot"))]
    let arguments_limit = 3;
    #[cfg(feature = "u-boot")]
    let arguments_limit = 2;

    // check the function arguments
    if f.sig.inputs.len() > arguments_limit {
        return parse::Error::new(
            f.sig.inputs.last().unwrap().span(),
            "`#[entry]` function has too many arguments",
        )
        .to_compile_error()
        .into();
    }

    fn check_correct_type(argument: &PatType, ty: &str) -> Option<TokenStream> {
        let inv_type_message = format!("argument type must be {}", ty);

        if !is_correct_type(&argument.ty, ty) {
            let error = parse::Error::new(argument.ty.span(), inv_type_message);

            Some(error.to_compile_error().into())
        } else {
            None
        }
    }
    fn check_argument_type(argument: &FnArg, ty: &str) -> Option<TokenStream> {
        let argument_error = parse::Error::new(argument.span(), "invalid argument");
        let argument_error = argument_error.to_compile_error().into();

        match argument {
            FnArg::Typed(argument) => check_correct_type(argument, ty),
            FnArg::Receiver(_) => Some(argument_error),
        }
    }
    #[cfg(not(feature = "u-boot"))]
    for argument in f.sig.inputs.iter() {
        if let Some(message) = check_argument_type(argument, "usize") {
            return message;
        };
    }
    #[cfg(feature = "u-boot")]
    if let Some(argument) = f.sig.inputs.get(0) {
        if let Some(message) = check_argument_type(argument, "c_int") {
            return message;
        }
    }
    #[cfg(feature = "u-boot")]
    if let Some(argument) = f.sig.inputs.get(1) {
        if let Some(message) = check_argument_type(argument, "*const *const c_char") {
            return message;
        }
    }

    // check the function signature
    let valid_signature = f.sig.constness.is_none()
        && f.sig.asyncness.is_none()
        && f.vis == Visibility::Inherited
        && f.sig.abi.is_none()
        && f.sig.generics.params.is_empty()
        && f.sig.generics.where_clause.is_none()
        && f.sig.variadic.is_none()
        && match f.sig.output {
            ReturnType::Default => false,
            ReturnType::Type(_, ref ty) => matches!(**ty, Type::Never(_)),
        };

    if !valid_signature {
        return parse::Error::new(
            f.span(),
            "`#[entry]` function must have signature `[unsafe] fn([arg0: usize, ...]) -> !`",
        )
        .to_compile_error()
        .into();
    }

    if !args.is_empty() {
        return parse::Error::new(Span::call_site(), "This attribute accepts no arguments")
            .to_compile_error()
            .into();
    }

    // XXX should we blacklist other attributes?
    let attrs = f.attrs;
    let unsafety = f.sig.unsafety;
    let args = f.sig.inputs;
    let stmts = f.block.stmts;

    quote!(
        #[allow(non_snake_case)]
        #[export_name = "main"]
        #(#attrs)*
        pub #unsafety fn __risc_v_rt__main(#args) -> ! {
            #(#stmts)*
        }
    )
    .into()
}

fn strip_type_path(ty: &Type) -> Option<Type> {
    match ty {
        Type::Ptr(ty) => {
            let mut ty = ty.clone();
            ty.elem = Box::new(strip_type_path(&ty.elem)?);
            Some(Type::Ptr(ty))
        }
        Type::Path(ty) => {
            let mut ty = ty.clone();
            let last_segment = ty.path.segments.last().unwrap().clone();
            ty.path.segments = Punctuated::new();
            ty.path.segments.push_value(last_segment);
            Some(Type::Path(ty))
        }
        _ => None,
    }
}

#[allow(unused)]
fn is_correct_type(ty: &Type, name: &str) -> bool {
    let correct: Type = syn::parse_str(name).unwrap();
    if let Some(ty) = strip_type_path(ty) {
        ty == correct
    } else {
        false
    }
}

/// Attribute to mark which function will be called at the beginning of the reset handler.
///
/// **IMPORTANT**: This attribute can appear at most *once* in the dependency graph. Also, if you
/// are using Rust 1.30 the attribute must be used on a reachable item (i.e. there must be no
/// private modules between the item and the root of the crate); if the item is in the root of the
/// crate you'll be fine. This reachability restriction doesn't apply to Rust 1.31 and newer
/// releases.
///
/// The function must have the signature of `unsafe fn()`.
///
/// The function passed will be called before static variables are initialized. Any access of static
/// variables will result in undefined behavior.
///
/// # Examples
///
/// ```
/// # use riscv_rt_macros::pre_init;
/// #[pre_init]
/// unsafe fn before_main() {
///     // do something here
/// }
///
/// # fn main() {}
/// ```
#[proc_macro_attribute]
pub fn pre_init(args: TokenStream, input: TokenStream) -> TokenStream {
    let f = parse_macro_input!(input as ItemFn);

    // check the function signature
    let valid_signature = f.sig.constness.is_none()
        && f.sig.asyncness.is_none()
        && f.vis == Visibility::Inherited
        && f.sig.unsafety.is_some()
        && f.sig.abi.is_none()
        && f.sig.inputs.is_empty()
        && f.sig.generics.params.is_empty()
        && f.sig.generics.where_clause.is_none()
        && f.sig.variadic.is_none()
        && match f.sig.output {
            ReturnType::Default => true,
            ReturnType::Type(_, ref ty) => match **ty {
                Type::Tuple(ref tuple) => tuple.elems.is_empty(),
                _ => false,
            },
        };

    if !valid_signature {
        return parse::Error::new(
            f.span(),
            "`#[pre_init]` function must have signature `unsafe fn()`",
        )
        .to_compile_error()
        .into();
    }

    if !args.is_empty() {
        return parse::Error::new(Span::call_site(), "This attribute accepts no arguments")
            .to_compile_error()
            .into();
    }

    // XXX should we blacklist other attributes?
    let attrs = f.attrs;
    let ident = f.sig.ident;
    let block = f.block;

    quote!(
        #[export_name = "__pre_init"]
        #(#attrs)*
        pub unsafe fn #ident() #block
    )
    .into()
}

struct AsmLoopArgs {
    asm_template: String,
    count_from: usize,
    count_to: usize,
}

impl Parse for AsmLoopArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let template: LitStr = input.parse().unwrap();
        _ = input.parse::<Token![,]>().unwrap();
        let count: LitInt = input.parse().unwrap();
        if input.parse::<Token![,]>().is_ok() {
            let count_to: LitInt = input.parse().unwrap();
            Ok(Self {
                asm_template: template.value(),
                count_from: count.base10_parse().unwrap(),
                count_to: count_to.base10_parse().unwrap(),
            })
        } else {
            Ok(Self {
                asm_template: template.value(),
                count_from: 0,
                count_to: count.base10_parse().unwrap(),
            })
        }
    }
}

/// Loops an asm expression n times.
///
/// `loop_asm!` takes 2 or 3 arguments, the first is a string literal and the rest are a number literal
/// See [the formatting syntax documentation in `std::fmt`](../std/fmt/index.html) for details.
///
/// Argument 1 is an assembly expression, all "{}" in this assembly expression will be replaced with the
/// current loop index.
///
/// If 2 arguments are provided, the loop will start at 0 and end at the number provided in argument 2.
///
/// If 3 arguments are provided, the loop will start at the number provided in argument 2 and end at
/// the number provided in argument 3.
///
/// # Examples
///
/// ```
/// # use riscv_rt_macros::loop_asm;
/// unsafe {
///     loop_asm!("fmv.w.x f{}, x0", 32); // => core::arch::asm!("fmv.w.x f0, x0") ... core::arch::asm!("fmv.w.x f31, x0")
///     loop_asm!("fmv.w.x f{}, x0", 1, 32); // => core::arch::asm!("fmv.w.x f1, x0") ... core::arch::asm!("fmv.w.x f31, x0")
/// }
/// ```
#[proc_macro]
pub fn loop_asm(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as AsmLoopArgs);

    let tokens = (args.count_from..args.count_to)
        .map(|i| {
            let i = i.to_string();
            let asm = args.asm_template.replace("{}", &i);
            format!("core::arch::asm!(\"{}\");", asm)
        })
        .collect::<Vec<String>>()
        .join("\n");
    tokens.parse().unwrap()
}

/// Loops a global_asm expression n times.
///
/// `loop_global_asm!` takes 2 or 3 arguments, the first is a string literal and the rest are a number literal
/// See [the formatting syntax documentation in `std::fmt`](../std/fmt/index.html) for details.
///
/// Argument 1 is an assembly expression, all "{}" in this assembly expression will be replaced with the
/// current loop index.
///
/// If 2 arguments are provided, the loop will start at 0 and end at the number provided in argument 2.
///
/// If 3 arguments are provided, the loop will start at the number provided in argument 2 and end at
/// the number provided in argument 3.
///
/// # Examples
///
/// ```
/// # use riscv_rt_macros::loop_global_asm;
/// unsafe {
///     loop_global_asm!("fmv.w.x f{}, x0", 32); // => core::arch::global_asm!("fmv.w.x f0, x0") ... core::arch::global_asm!("fmv.w.x f31, x0")
///     loop_global_asm!("fmv.w.x f{}, x0", 1, 32); // => core::arch::global_asm!("fmv.w.x f1, x0") ... core::arch::global_asm!("fmv.w.x f31, x0")
/// }
/// ```
#[proc_macro]
pub fn loop_global_asm(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as AsmLoopArgs);

    let instructions = (args.count_from..args.count_to)
        .map(|i| {
            let i = i.to_string();
            args.asm_template.replace("{}", &i)
        })
        .collect::<Vec<String>>()
        .join("\n");

    let res = format!("core::arch::global_asm!(\n\"{}\"\n);", instructions);
    res.parse().unwrap()
}

#[derive(Clone, Copy, Debug)]
enum RiscvArch {
    Rv32,
    Rv64,
}

impl RiscvArch {
    fn width(&self) -> usize {
        match self {
            Self::Rv32 => 4,
            Self::Rv64 => 8,
        }
    }

    fn store(&self) -> &str {
        match self {
            Self::Rv32 => "sw",
            Self::Rv64 => "sd",
        }
    }

    fn load(&self) -> &str {
        match self {
            Self::Rv32 => "lw",
            Self::Rv64 => "ld",
        }
    }
}

/// Size of the trap frame (in number of registers)
const TRAP_SIZE: usize = 16;

#[rustfmt::skip]
/// List of the register names to be stored in the trap frame
const TRAP_FRAME: [&str; TRAP_SIZE] = [
    "ra",
    "t0",
    "t1",
    "t2",
    "t3",
    "t4",
    "t5",
    "t6",
    "a0",
    "a1",
    "a2",
    "a3",
    "a4",
    "a5",
    "a6",
    "a7",
];

/// Generate the assembly instructions to store the trap frame.
///
/// The `arch` parameter is used to determine the width of the registers.
///
/// The `filter` function is used to filter which registers to store.
/// This is useful to optimize the binary size in vectored interrupt mode, which divides the trap
/// frame storage in two parts: the first part saves space in the stack and stores only the `a0` register,
/// while the second part stores the remaining registers.
fn store_trap<T: FnMut(&str) -> bool>(arch: RiscvArch, mut filter: T) -> String {
    let width = arch.width();
    let store = arch.store();
    TRAP_FRAME
        .iter()
        .enumerate()
        .filter(|(_, &reg)| filter(reg))
        .map(|(i, reg)| format!("{store} {reg}, {i}*{width}(sp)"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate the assembly instructions to load the trap frame.
/// The `arch` parameter is used to determine the width of the registers.
fn load_trap(arch: RiscvArch) -> String {
    let width = arch.width();
    let load = arch.load();
    TRAP_FRAME
        .iter()
        .enumerate()
        .map(|(i, reg)| format!("{load} {reg}, {i}*{width}(sp)"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generates weak `_start_trap` function in assembly for RISCV-32 targets.
///
/// This implementation stores all registers in the trap frame and calls `_start_trap_rust`.
/// The trap frame is allocated on the stack and deallocated after the call.
#[proc_macro]
pub fn weak_start_trap_riscv32(_input: TokenStream) -> TokenStream {
    weak_start_trap(RiscvArch::Rv32)
}

/// Generates weak `_start_trap` function in assembly for RISCV-64 targets.
///
/// This implementation stores all registers in the trap frame and calls `_start_trap_rust`.
/// The trap frame is allocated on the stack and deallocated after the call.
#[proc_macro]
pub fn weak_start_trap_riscv64(_input: TokenStream) -> TokenStream {
    weak_start_trap(RiscvArch::Rv64)
}

/// Generates weak `_start_trap` function in assembly.
///
/// This implementation stores all registers in the trap frame and calls `_start_trap_rust`.
/// The trap frame is allocated on the stack and deallocated after the call.
///
/// The `arch` parameter is used to determine the width of the registers.
/// The macro also ensures that the trap frame size is 16-byte aligned.
fn weak_start_trap(arch: RiscvArch) -> TokenStream {
    let width = arch.width();
    // ensure we do not break that sp is 16-byte aligned
    if (TRAP_SIZE * width) % 16 != 0 {
        return parse::Error::new(Span::call_site(), "Trap frame size must be 16-byte aligned")
            .to_compile_error()
            .into();
    }
    let store = store_trap(arch, |_| true);
    let load = load_trap(arch);

    #[cfg(feature = "s-mode")]
    let ret = "sret";
    #[cfg(not(feature = "s-mode"))]
    let ret = "mret";

    format!(
        r#"
core::arch::global_asm!(
".section .trap, \"ax\"
.align {width}
.weak _start_trap
_start_trap:
    addi sp, sp, - {TRAP_SIZE} * {width}
    {store}
    add a0, sp, zero
    jal ra, _start_trap_rust
    {load}
    addi sp, sp, {TRAP_SIZE} * {width}
    {ret}
");"#
    )
    .parse()
    .unwrap()
}

/// Generates vectored interrupt trap functions in assembly for RISCV-32 targets.
#[cfg(feature = "v-trap")]
#[proc_macro]
pub fn vectored_interrupt_trap_riscv32(_input: TokenStream) -> TokenStream {
    vectored_interrupt_trap(RiscvArch::Rv32)
}

/// Generates vectored interrupt trap functions in assembly for RISCV-64 targets.
#[cfg(feature = "v-trap")]
#[proc_macro]
pub fn vectored_interrupt_trap_riscv64(_input: TokenStream) -> TokenStream {
    vectored_interrupt_trap(RiscvArch::Rv64)
}

#[cfg(feature = "v-trap")]
/// Generates global '_start_DefaultHandler_trap' and '_continue_interrupt_trap' functions in assembly.
/// The '_start_DefaultHandler_trap' function stores the trap frame partially (only register a0) and
/// jumps to the interrupt handler. The '_continue_interrupt_trap' function stores the trap frame
/// partially (all registers except a0), jumps to the interrupt handler, and restores the trap frame.
fn vectored_interrupt_trap(arch: RiscvArch) -> TokenStream {
    let width = arch.width();
    let store_start = store_trap(arch, |reg| reg == "a0");
    let store_continue = store_trap(arch, |reg| reg != "a0");
    let load = load_trap(arch);

    #[cfg(feature = "s-mode")]
    let ret = "sret";
    #[cfg(not(feature = "s-mode"))]
    let ret = "mret";

    let instructions = format!(
        r#"
core::arch::global_asm!(
".section .trap, \"ax\"

.align 4
.global _start_DefaultHandler_trap
_start_DefaultHandler_trap:
    addi sp, sp, -{TRAP_SIZE} * {width} // allocate space for trap frame
    {store_start}                       // store trap partially (only register a0)
    la a0, DefaultHandler               // load interrupt handler address into a0

.align 4
.global _continue_interrupt_trap
_continue_interrupt_trap:
    {store_continue}                   // store trap partially (all registers except a0)
    jalr ra, a0, 0                     // jump to corresponding interrupt handler (address stored in a0)
    {load}                             // restore trap frame
    addi sp, sp, {TRAP_SIZE} * {width} // deallocate space for trap frame
    {ret}                              // return from interrupt
");"#
    );

    instructions.parse().unwrap()
}

#[derive(Clone, Copy, Debug)]
enum RiscvPacItem {
    Exception,
    ExternalInterrupt,
    CoreInterrupt,
}

impl RiscvPacItem {
    fn macro_id(&self) -> &str {
        match self {
            Self::Exception => "exception",
            Self::ExternalInterrupt => "external_interrupt",
            Self::CoreInterrupt => "core_interrupt",
        }
    }

    fn valid_signature(&self) -> &str {
        match self {
            Self::Exception => "`[unsafe] fn([&[mut] riscv_rt::TrapFrame]) [-> !]`",
            _ => "`[unsafe] fn() [-> !]`",
        }
    }

    fn check_signature(&self, f: &ItemFn) -> bool {
        let valid_args = match self {
            Self::Exception => {
                if f.sig.inputs.len() > 1 {
                    return false;
                }
                match f.sig.inputs.first() {
                    Some(FnArg::Typed(t)) => {
                        let first_param_type = *t.ty.clone();
                        let expected_types: Vec<Type> = vec![
                            parse_quote!(&riscv_rt::TrapFrame),
                            parse_quote!(&mut riscv_rt::TrapFrame),
                        ];
                        expected_types.iter().any(|t| first_param_type == *t)
                    }
                    Some(_) => false,
                    None => true,
                }
            }
            _ => f.sig.inputs.is_empty(),
        };

        valid_args
            && f.sig.constness.is_none()
            && f.sig.asyncness.is_none()
            && f.vis == Visibility::Inherited
            && f.sig.abi.is_none()
            && f.sig.generics.params.is_empty()
            && f.sig.generics.where_clause.is_none()
            && f.sig.variadic.is_none()
            && match f.sig.output {
                ReturnType::Default => true,
                ReturnType::Type(_, ref ty) => matches!(**ty, Type::Never(_)),
            }
    }

    fn impl_trait(&self) -> TokenStream2 {
        match self {
            Self::Exception => quote! { riscv_rt::ExceptionNumber },
            Self::ExternalInterrupt => quote! { riscv_rt::ExternalInterruptNumber },
            Self::CoreInterrupt => quote! { riscv_rt::CoreInterruptNumber },
        }
    }
}

#[proc_macro_attribute]
/// Attribute to declare an exception handler.
///
/// The function must have the signature `[unsafe] fn([&[mut] riscv_rt::TrapFrame]) [-> !]`.
///
/// The argument of the macro must be a path to a variant of an enum that implements the `riscv_rt::ExceptionNumber` trait.
///
/// # Example
///
/// ``` ignore,no_run
/// #[riscv_rt::exception(riscv::interrupt::Exception::LoadMisaligned)]
/// fn load_misaligned(trap_frame: &mut riscv_rt::TrapFrame) -> ! {
///     loop{};
/// }
/// ```
pub fn exception(args: TokenStream, input: TokenStream) -> TokenStream {
    trap(args, input, RiscvPacItem::Exception, None)
}

#[proc_macro_attribute]
/// Attribute to declare a core interrupt handler.
///
/// The function must have the signature `[unsafe] fn() [-> !]`.
///
/// The argument of the macro must be a path to a variant of an enum that implements the `riscv_rt::CoreInterruptNumber` trait.
///
/// If the `v-trap` feature is enabled, this macro generates the corresponding interrupt trap handler in assembly.
///
/// # Example
///
/// ``` ignore,no_run
/// #[riscv_rt::core_interrupt(riscv::interrupt::Interrupt::SupervisorSoft)]
/// fn supervisor_soft() -> ! {
///     loop{};
/// }
/// ```
pub fn core_interrupt_riscv32(args: TokenStream, input: TokenStream) -> TokenStream {
    let arch = match () {
        #[cfg(feature = "v-trap")]
        () => Some(RiscvArch::Rv32),
        #[cfg(not(feature = "v-trap"))]
        () => None,
    };
    trap(args, input, RiscvPacItem::CoreInterrupt, arch)
}

#[proc_macro_attribute]
/// Attribute to declare a core interrupt handler.
///
/// The function must have the signature `[unsafe] fn() [-> !]`.
///
/// The argument of the macro must be a path to a variant of an enum that implements the `riscv_rt::CoreInterruptNumber` trait.
///
/// If the `v-trap` feature is enabled, this macro generates the corresponding interrupt trap handler in assembly.
///
/// # Example
///
/// ``` ignore,no_run
/// #[riscv_rt::core_interrupt(riscv::interrupt::Interrupt::SupervisorSoft)]
/// fn supervisor_soft() -> ! {
///     loop{};
/// }
/// ```
pub fn core_interrupt_riscv64(args: TokenStream, input: TokenStream) -> TokenStream {
    let arch = match () {
        #[cfg(feature = "v-trap")]
        () => Some(RiscvArch::Rv64),
        #[cfg(not(feature = "v-trap"))]
        () => None,
    };
    trap(args, input, RiscvPacItem::CoreInterrupt, arch)
}

#[proc_macro_attribute]
/// Attribute to declare an external interrupt handler.
///
/// The function must have the signature `[unsafe] fn() [-> !]`.
///
/// The argument of the macro must be a path to a variant of an enum that implements the `riscv_rt::ExternalInterruptNumber` trait.
///
/// # Example
///
/// ``` ignore,no_run
/// #[riscv_rt::external_interrupt(e310x::interrupt::Interrupt::GPIO0)]
/// fn gpio0() -> ! {
///     loop{};
/// }
/// ```
pub fn external_interrupt(args: TokenStream, input: TokenStream) -> TokenStream {
    trap(args, input, RiscvPacItem::ExternalInterrupt, None)
}

fn trap(
    args: TokenStream,
    input: TokenStream,
    pac_item: RiscvPacItem,
    arch: Option<RiscvArch>,
) -> TokenStream {
    let f = parse_macro_input!(input as ItemFn);

    if !pac_item.check_signature(&f) {
        let msg = format!(
            "`#[{}]` function must have signature {}",
            pac_item.macro_id(),
            pac_item.valid_signature()
        );
        return parse::Error::new(f.sig.span(), msg)
            .to_compile_error()
            .into();
    }
    if args.is_empty() {
        let msg = format!(
            "`#[{}]` attribute expects a path to a variant of an enum that implements the {} trait.",
            pac_item.macro_id(),
            pac_item.impl_trait()
        );
        return parse::Error::new(Span::call_site(), msg)
            .to_compile_error()
            .into();
    }

    let int_path = parse_macro_input!(args as Path);
    let int_ident = &int_path.segments.last().unwrap().ident;
    let export_name = format!("{:#}", int_ident);

    let start_trap = match arch {
        Some(arch) => {
            let trap = start_interrupt_trap(int_ident, arch);
            quote! {
                #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
                #trap
            }
        }
        None => proc_macro2::TokenStream::new(),
    };

    let pac_trait = pac_item.impl_trait();

    quote!(
        // Compile-time check to ensure the trap path implements the trap trait
        const _: fn() = || {
            fn assert_impl<T: #pac_trait>(_arg: T) {}
            assert_impl(#int_path);
        };

        #start_trap

        #[export_name = #export_name]
        #f
    )
    .into()
}

fn start_interrupt_trap(ident: &syn::Ident, arch: RiscvArch) -> proc_macro2::TokenStream {
    let interrupt = ident.to_string();
    let width = arch.width();
    let store = store_trap(arch, |r| r == "a0");

    let instructions = format!(
        r#"
core::arch::global_asm!(
    ".section .trap, \"ax\"
    .align 2
    .global _start_{interrupt}_trap
    _start_{interrupt}_trap:
        addi sp, sp, -{TRAP_SIZE} * {width} // allocate space for trap frame
        {store}                             // store trap partially (only register a0)
        la a0, {interrupt}                  // load interrupt handler address into a0
        j _continue_interrupt_trap          // jump to common part of interrupt trap
");"#
    );

    instructions.parse().unwrap()
}
