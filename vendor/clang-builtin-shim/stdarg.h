#ifndef _MUSIALIZER_SHIM_STDARG_H
#define _MUSIALIZER_SHIM_STDARG_H
typedef __builtin_va_list va_list;
typedef __builtin_va_list __gnuc_va_list;
#define va_start(ap, param) __builtin_va_start(ap, param)
#define va_end(ap) __builtin_va_end(ap)
#define va_arg(ap, type) __builtin_va_arg(ap, type)
#define va_copy(dst, src) __builtin_va_copy(dst, src)
#define _VA_LIST_DEFINED
#define _VA_LIST
#endif
