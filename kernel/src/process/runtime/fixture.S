// SPDX-License-Identifier: Apache-2.0
// Original native test applications, embedded as ELF files in boot-image rodata.
// They are copied by the same untrusted ELF loader used for future boot modules.
.section .rodata
.macro APPLICATION name, data_address
.balign 16
.global \name
\name:
    .byte 0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0, 0
    .zero 7
    .short 2, 62
    .long 1
    .quad 0x400000, 64, 0
    .long 0
    .short 64, 56, 2, 0, 0, 0
    .long 1, 5
    .quad 4096, 0x400000, 0, .Lcode_end\@ - .Lcode\@, .Lcode_end\@ - .Lcode\@, 4096
    .long 1, 6
    .quad 8192, \data_address, 0, 8, 4096, 4096
    .zero 4096 - (. - \name)
.Lcode\@:
    mov r13, rdi
    mov r14, rsi
    mov r15, rdx
    mov ebx, \data_address
    test rsp, 15
    jnz .Lfail\@
    mov ax, cs
    cmp ax, 0x2b
    jne .Lfail\@
    mov ax, ss
    cmp ax, 0x33
    jne .Lfail\@
    cmp qword ptr [rbx], 0x11223344
    jne .Lfail\@
    mov ecx, 8
.Lbss\@:
    cmp qword ptr [rbx + rcx], 0
    jne .Lfail\@
    add ecx, 8
    cmp ecx, 4096
    jne .Lbss\@
    mov [rbx + 8], r15
    mov r8d, 8
    mov r9d, 9
    mov r10d, 10
    mov r11d, 11
    mov ecx, 12
    mov edx, 13
    mov esi, 14
    mov ebp, 15
    std
    xor eax, eax
    int 128
    cmp eax, 0x10000
    jne .Lfail\@
    pushfq
    pop rax
    cld
    test eax, 0x400
    jz .Lfail\@
    test eax, 0x3000
    jnz .Lfail\@
    cmp r8d, 8
    jne .Lfail\@
    cmp r9d, 9
    jne .Lfail\@
    cmp r10d, 10
    jne .Lfail\@
    cmp r11d, 11
    jne .Lfail\@
    cmp ecx, 12
    jne .Lfail\@
    cmp edx, 13
    jne .Lfail\@
    cmp esi, 14
    jne .Lfail\@
    cmp ebp, 15
    jne .Lfail\@
    mov eax, 3
    int 128
    test rax, rax
    jz .Lfail\@
    mov [rbx + 16], rax
    mov eax, 0xffff
    int 128
    cmp rax, -1
    jne .Lfail\@
    cmp r13, 0
    je .Lworker\@
    cmp r13, 1
    je .Lspin\@
    cmp r13, 2
    je .Lread\@
    cmp r13, 3
    je .Lwrite\@
    cmp r13, 4
    je .Lexecute\@
    cmp r13, 5
    je .Lcli\@
    cmp r13, 6
    je .Lud\@
    cmp r13, 7
    je .Lfp\@
    cmp r13, 8
    je .Lgate\@
    cmp r13, 9
    je .Lstack\@
    cmp r13, 10
    je .Lquota\@
    cmp r13, 11
    je .Lsyscall\@
    cmp r13, 12
    je .Lport\@
    cmp r13, 13
    je .Lhalt\@
    jmp .Lfail\@
.Lworker\@:
    mov eax, 2
    mov rdi, r15
    int 128
    test rax, rax
    jnz .Lfail\@
    cmp [rbx + 8], r15
    jne .Lfail\@
    mov eax, 3
    int 128
    cmp [rbx + 16], rax
    jne .Lfail\@
    mov rdi, r15
    mov eax, 1
    int 128
    ud2
.Lspin\@:
    cmp [rbx + 8], r15
    jne .Lfail\@
    inc qword ptr [rbx + 24]
    inc r12
    jmp .Lspin\@
.Lread\@:
    mov rax, [r14]
    jmp .Lfail\@
.Lwrite\@:
    mov [r14], r15
    jmp .Lfail\@
.Lexecute\@:
    call r14
    jmp .Lfail\@
.Lcli\@:
    cli
    jmp .Lfail\@
.Lud\@:
    ud2
.Lfp\@:
    fld1
    jmp .Lfail\@
.Lgate\@:
    int 129
    jmp .Lfail\@
.Lstack\@:
    mov rsp, 0x0000800000000000
    mov eax, 0
    int 128
    jmp .Lfail\@
.Lsyscall\@:
    syscall
    jmp .Lfail\@
.Lport\@:
    mov dx, 0x80
    out dx, al
    jmp .Lfail\@
.Lhalt\@:
    hlt
    jmp .Lfail\@
.Lquota\@:
    mov r12d, 8
.Lreports\@:
    mov eax, 2
    mov rdi, r15
    int 128
    test rax, rax
    jnz .Lfail\@
    dec r12d
    jnz .Lreports\@
    mov eax, 2
    int 128
    cmp rax, -2
    jne .Lfail\@
    mov eax, 1
    mov rdi, r15
    int 128
.Lfail\@:
    mov eax, 1
    mov edi, 0xbad
    int 128
    ud2
.Lcode_end\@:
    .zero 8192 - (. - \name)
    .quad 0x11223344
.global \name\()_end
\name\()_end:
.endm
APPLICATION rustic_user_elf, 0x600000
APPLICATION rustic_victim_elf, 0x700000
