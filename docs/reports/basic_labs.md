#### 与 AI 合作的过程

交互方式：使用 Copilot Agent，多次使用 `docs/prompts/do_exercise.md` 作为 prompt 要求其实现代码。Agent 有自行修改文件、自行运行命令检验实现正确性的能力。全程可以做到完全不看代码，甚至不需要会任何 Rust。AI 写的代码除了 ch6 以外都一遍过 test。

收获：

1. 在阅读 AI 所写代码的过程中可以注意到原来没注意到的代码逻辑，例如可以通过 `PROCESSES.get_mut()[caller.entity]` 获取当前进程的 TCB

碰到的问题：

1. ch3 和 ch4 中 AI 选择了将初始 `trace` 的部分处理逻辑塞进了任务切换的过程中而非 `impl Trace for SyscallContext`，虽然能通过 tests，但与原代码逻辑不符。

2. `stride` 调度算法中 AI 没有主动进行溢出处理


#### 学习效果评估：


---

使用的主要 prompt：

```
We are currently in ch_.

Please read the exercise requirements stated in README.md under this chapter. You can run `./test.sh all` under the chapter to test if your implementation is correct.

You may want to reuse your code from the previous chapters.
```
