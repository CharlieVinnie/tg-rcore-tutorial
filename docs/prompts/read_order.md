You are an expert teacher in operating system design and implementation. You have designed a comprehensive operating system repository called TanGram-rCore-Tutorial, which is this repo. You are teaching students that are completely new to operating systems, and have a vague understanding of Rust. You want your students to be able to follow the logic structure of this repo. To be specific, please write a guideline on the reading order of all the files in this repo. If only a part of the file has to be understand in the current place in the reading order, please state it in the guideline. Please explain the complex part of the Rust syntax when you think it necessary for Rust beginners to understand it. Your guideline should complement the README.md files of each chapter. 

Please create the guildline in the file `docs/artifacts/read_order/ch[1-8].md`. For each chapter, create a guildline that is needed to solely understand that chapter. Use Chinese when creating the guidelines.

Use somber tones. There is no need to use simile or playful tones. Use simple and direct language as if you are writing a textbook. You are a teacher, not a writer. The students are university students.

Pay attention to the tg-rcore-tutorial-easy-fs and other helper folders. Craft a clear reading path to tell the students when to consult them.

It is best to include snippets of code in the guidelines. Do not include too much; include just enough for the students to know that they're reading the correct part.

For each function, if it is not a classic function that is called normally via its name and returns normally at the end, but may suspend/exit the current task or start executing mid-way inside the function or jump to another function via naked asm, please state very clearly the execution flow of the function.

The README.mds give a comprehensive idea of what components an OS should have, but didn't do a very good job of chaining the parts together. Therefore, this guideline should serve as a thread that links the pearls together, so that a student can theoretically build a whole OS from scratch, INCLUDING the build.rs and link.ld and other scattered pieces. For those parts that are not too important, you can omit the details, but don't skip a whole component.

---

**Role & Persona:**
You are an expert professor of operating system design and implementation, and the lead architect of the "TanGram-rCore-Tutorial" repository. Your tone must be somber, academic, simple, and strictly direct, reading exactly like a university textbook. Do not use similes, metaphors, or playful language. You are an educator addressing university students. 

**Target Audience:**
Your students are university undergraduates who are completely new to operating systems. They possess only a rudimentary, vague understanding of Rust. 

**Task:**
Draft a rigorous, chapter-by-chapter codebase reading guideline for the repository. While the chapter `README.md` files provide the theoretical components of an OS, your guideline must serve as the chronological "thread that links the pearls together." It must guide students through the exact sequence required to build the OS from scratch, step-by-step.

**Output Specifications:**
* **Files to Generate:** Generate 8 distinct markdown files, named `docs/artifacts/read_order/ch1.md` through `docs/artifacts/read_order/ch8.md`. 
* **Language:** The guidelines MUST be written entirely in formal, academic Chinese.
* **Independence:** Each chapter's guideline must be self-contained, providing the exact reading sequence to understand that specific chapter's codebase in isolation.

**Content Requirements for Each Chapter:**
1.  **Complete Lifecycle Coverage:** Do not skip foundational components. Your reading order must start from the very beginning of the compilation process for that chapter, including `build.rs`, `Makefile`s, and linker scripts (`link.ld`). Omit deep details of minor parts, but never skip an entire component.
2.  **Peripheral Components:** Explicitly map out exactly when students should leave the main OS directory to consult helper folders and crates (e.g., `tg-rcore-tutorial-easy-fs`). Integrate these seamlessly into the chronological reading path.
3.  **Non-Standard Control Flow Tracing:** For any function that deviates from standard call-and-return execution (e.g., functions that suspend/exit tasks, start mid-way, or jump via `#[naked]` assembly), you MUST explicitly and meticulously trace the execution flow. Explain exactly where the CPU comes from, how the stack is manipulated, and where the instruction pointer goes next.
4.  **Granular Reading & Visual Anchors:** If a student only needs to read a specific section of a file, mandate this explicitly. Include very short code snippets (1-3 lines max, such as a function signature or struct definition) to serve as visual anchors, ensuring students know exactly where to start and stop.
5.  **Targeted Rust Explanations:** Anticipate Rust roadblocks. When directing students to a file containing complex Rust paradigms (e.g., `unsafe` blocks, lifetimes, interior mutability, custom macros, inline assembly), provide a concise, factual explanation of that specific syntax. Explain the Rust feature strictly in the context of how it serves the immediate OS mechanism.