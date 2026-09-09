// SPDX-License-Identifier: Apache-2.0
// Original ELF application exercising the public integer/buffer ABI from ring 3.
.section .rodata
.balign 16
.global rustic_ipc_elf
rustic_ipc_elf:
    .byte 0x7f,0x45,0x4c,0x46,2,1,1,0,0
    .zero 7
    .short 2,62
    .long 1
    .quad 0x400000,64,0
    .long 0
    .short 64,56,2,0,0,0
    .long 1,5
    .quad 4096,0x400000,0,.Lend-.Lcode,.Lend-.Lcode,4096
    .long 1,6
    .quad 8192,0x600000,0,8,4096,4096
    .zero 4096-(.-rustic_ipc_elf)
.Lcode:
    mov r12,rdi
    mov r13,rsi
    mov r14,rdx
    mov ebx,0x600100
    cmp r12,18
    jne .Lpacket
    mov ebx,0x7fffeff0
.Lpacket:
    mov word ptr [rbx],1
    mov word ptr [rbx+2],1
    mov dword ptr [rbx+4],8
    mov qword ptr [rbx+8],123
    mov qword ptr [rbx+16],0
    mov qword ptr [rbx+24],0x42
    cmp r12,0
    je .Lsender
    cmp r12,1
    je .Lreceiver
    cmp r12,2
    je .Lwaiter
    cmp r12,3
    je .Lsend_address
    cmp r12,4
    je .Lsend_size
    cmp r12,5
    je .Lrecv_address
    cmp r12,6
    je .Lrecv_size
    cmp r12,7
    je .Lversion
    cmp r12,8
    je .Lopcode
    cmp r12,9
    je .Lspoof
    cmp r12,10
    je .Lstale
    cmp r12,11
    je .Lfull
    cmp r12,14
    je .Linfo
    cmp r12,19
    je .Lwait_close
    jmp .Lsend_once
.Lsender:
    call .Lsend
    test rax,rax
    jnz .Lexit
    call .Lreceive
    cmp rax,32
    jne .Lexit
    cmp [rbx+16],r14
    jne .Lbad
    cmp qword ptr [rbx+8],123
    jne .Lbad
    cmp qword ptr [rbx+24],0x84
    jne .Lbad
    mov eax,42
    jmp .Lexit
.Lreceiver:
    call .Lreceive
    cmp rax,32
    jne .Lexit
    cmp [rbx+16],r14
    jne .Lbad
    cmp qword ptr [rbx+24],0x42
    jne .Lbad
    mov qword ptr [rbx+16],0
    mov qword ptr [rbx+24],0x84
    call .Lsend
    test rax,rax
    jnz .Lexit
    mov eax,43
    jmp .Lexit
.Lwaiter:
    mov eax,6
    mov rdi,r13
    int 128
    jmp .Lexit
.Lsend_address:
    mov eax,4
    mov rdi,r13
    mov rsi,r14
    mov edx,32
    int 128
    jmp .Lexit
.Lsend_size:
    mov eax,4
    mov rdi,r13
    mov rsi,rbx
    mov rdx,r14
    int 128
    jmp .Lexit
.Lrecv_address:
    mov eax,5
    mov rdi,r13
    mov rsi,r14
    mov edx,32
    int 128
    jmp .Lexit
.Lrecv_size:
    mov eax,5
    mov rdi,r13
    mov rsi,rbx
    mov rdx,r14
    int 128
    jmp .Lexit
.Lversion:
    mov [rbx],r14w
    jmp .Lsend_once
.Lopcode:
    mov [rbx+2],r14w
    jmp .Lsend_once
.Lspoof:
    mov [rbx+16],r14
    jmp .Lsend_once
.Lstale:
    mov eax,7
    mov rdi,r13
    int 128
    test rax,rax
    jnz .Lexit
    jmp .Lsend_once
.Lwait_close:
    mov eax,6
    mov rdi,r13
    int 128
    jmp .Lstale
.Lfull:
    mov r15d,3
.Lmore:
    call .Lsend
    inc qword ptr [rbx+8]
    dec r15d
    jnz .Lmore
    jmp .Lexit
.Linfo:
    mov eax,8
    int 128
    jmp .Lexit
.Lsend_once:
    call .Lsend
    jmp .Lexit
.Lsend:
    mov eax,4
    mov rdi,r13
    mov rsi,rbx
    mov edx,32
    int 128
    ret
.Lreceive:
    mov eax,6
    mov rdi,r13
    int 128
    test rax,rax
    jnz .Lreturn
    mov eax,5
    mov rdi,r13
    mov rsi,rbx
    mov edx,32
    int 128
.Lreturn:
    ret
.Lbad:
    mov eax,0xbad
.Lexit:
    mov rdi,rax
    mov eax,1
    int 128
    ud2
.Lend:
    .zero 8192-(.-rustic_ipc_elf)
    .quad 0
.global rustic_ipc_elf_end
rustic_ipc_elf_end:
