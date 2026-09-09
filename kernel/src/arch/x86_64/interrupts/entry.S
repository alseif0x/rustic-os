// SPDX-License-Identifier: Apache-2.0
// Original stubs: normalize hardware error codes; preserve GPRs and restore with IRETQ.
.section .text
.macro ISR number, error=0
.global rustic_isr_\number
rustic_isr_\number:
.if \error == 0
    push 0
.endif
    push \number
    jmp rustic_isr_common
.endm
ISR 0, 0
ISR 1, 0
ISR 2, 0
ISR 3, 0
ISR 4, 0
ISR 5, 0
ISR 6, 0
ISR 7, 0
ISR 8, 1
ISR 9, 0
ISR 10, 1
ISR 11, 1
ISR 12, 1
ISR 13, 1
ISR 14, 1
ISR 15, 0
ISR 16, 0
ISR 17, 1
ISR 18, 0
ISR 19, 0
ISR 20, 0
ISR 21, 1
ISR 22, 0
ISR 23, 0
ISR 24, 0
ISR 25, 0
ISR 26, 0
ISR 27, 0
ISR 28, 0
ISR 29, 1
ISR 30, 1
ISR 31, 0
ISR 32, 0
ISR 33, 0
ISR 34, 0
ISR 35, 0
ISR 36, 0
ISR 37, 0
ISR 38, 0
ISR 39, 0
ISR 40, 0
ISR 41, 0
ISR 42, 0
ISR 43, 0
ISR 44, 0
ISR 45, 0
ISR 46, 0
ISR 47, 0
ISR 128, 0
ISR 129, 0
.global rustic_user_roundtrip
rustic_user_roundtrip:
    int 129
    ret
.global rustic_isr_default
rustic_isr_default:
    push 0
    push 255
    jmp rustic_isr_common
rustic_isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    cld
    mov rdi, rsp
    mov rbx, rsp
    and rsp, -16
    call rustic_interrupt
    mov rsp, rbx
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    iretq
.section .rodata
.balign 8
.global rustic_isr_table
rustic_isr_table:
    .quad rustic_isr_0
    .quad rustic_isr_1
    .quad rustic_isr_2
    .quad rustic_isr_3
    .quad rustic_isr_4
    .quad rustic_isr_5
    .quad rustic_isr_6
    .quad rustic_isr_7
    .quad rustic_isr_8
    .quad rustic_isr_9
    .quad rustic_isr_10
    .quad rustic_isr_11
    .quad rustic_isr_12
    .quad rustic_isr_13
    .quad rustic_isr_14
    .quad rustic_isr_15
    .quad rustic_isr_16
    .quad rustic_isr_17
    .quad rustic_isr_18
    .quad rustic_isr_19
    .quad rustic_isr_20
    .quad rustic_isr_21
    .quad rustic_isr_22
    .quad rustic_isr_23
    .quad rustic_isr_24
    .quad rustic_isr_25
    .quad rustic_isr_26
    .quad rustic_isr_27
    .quad rustic_isr_28
    .quad rustic_isr_29
    .quad rustic_isr_30
    .quad rustic_isr_31
    .quad rustic_isr_32
    .quad rustic_isr_33
    .quad rustic_isr_34
    .quad rustic_isr_35
    .quad rustic_isr_36
    .quad rustic_isr_37
    .quad rustic_isr_38
    .quad rustic_isr_39
    .quad rustic_isr_40
    .quad rustic_isr_41
    .quad rustic_isr_42
    .quad rustic_isr_43
    .quad rustic_isr_44
    .quad rustic_isr_45
    .quad rustic_isr_46
    .quad rustic_isr_47
