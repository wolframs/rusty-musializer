#ifndef _MUSIALIZER_SHIM_STDDEF_H
#define _MUSIALIZER_SHIM_STDDEF_H
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;
typedef __WCHAR_TYPE__ wchar_t;
#define NULL ((void*)0)
#define offsetof(t, d) __builtin_offsetof(t, d)
typedef struct { long long __a; long double __b; } max_align_t;
#define __size_t__
#define __size_t
#define _SIZE_T
#define _SIZE_T_DEFINED
#define _SIZE_T_DECLARED
#define __SIZE_T__
#define _BSD_SIZE_T_DEFINED_
#define _PTRDIFF_T
#define _WCHAR_T
#define _STDDEF_H
#define _STDDEF_H_
#endif
