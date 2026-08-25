//! Hierarchy-Aware Mutator — memutasi subtree modul tertentu tanpa merusak project.

use super::expr::{BinOp, Expr};
use super::gen::GenInput;
use super::semantic_mutator::{
    GenerateBlock, GenerateKind, InstanceInfo, ModuleContext, SignalInfo, SignalKind, WidthKind,
};
use fastrand::Rng;

#[derive(Debug, Clone)]
pub struct HierarchyNode {
    pub path: String,
    pub module: ModuleContext,
    pub children: Vec<HierarchyNode>,
    pub parent: Option<String>,
}

impl HierarchyNode {
    pub fn new(path: String, module: ModuleContext) -> Self {
        HierarchyNode {
            path,
            module,
            children: Vec::new(),
            parent: None,
        }
    }

    pub fn add_child(&mut self, child: HierarchyNode) {
        self.children.push(child);
    }

    pub fn find_subtree(&self, target_path: &str) -> Option<&HierarchyNode> {
        if self.path == target_path {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_subtree(target_path) {
                return Some(found);
            }
        }
        None
    }

    pub fn find_subtree_mut(&mut self, target_path: &str) -> Option<&mut HierarchyNode> {
        if self.path == target_path {
            return Some(self);
        }
        for child in &mut self.children {
            if let Some(found) = child.find_subtree_mut(target_path) {
                return Some(found);
            }
        }
        None
    }

    pub fn all_paths(&self) -> Vec<String> {
        let mut paths = vec![self.path.clone()];
        for child in &self.children {
            paths.extend(child.all_paths());
        }
        paths
    }
}

#[derive(Clone)]
pub struct HierarchyMutator {
    rng: Rng,
    hierarchy: Option<HierarchyNode>,
    target_path: Option<String>,
}

impl HierarchyMutator {
    pub fn new(seed: u64) -> Self {
        HierarchyMutator {
            rng: Rng::with_seed(seed),
            hierarchy: None,
            target_path: None,
        }
    }

    pub fn with_hierarchy(mut self, root: HierarchyNode) -> Self {
        self.hierarchy = Some(root);
        self
    }

    pub fn has_hierarchy(&self) -> bool {
        self.hierarchy.is_some()
    }

    pub fn root_all_paths(&self) -> Option<Vec<String>> {
        self.hierarchy.as_ref().map(|h| h.all_paths())
    }

    pub fn target_subtree(mut self, path: &str) -> Self {
        self.target_path = Some(path.to_string());
        self
    }

    pub fn mutate_context(&mut self, ctx: &mut ModuleContext) {
        let seed = self.rng.u64(..);
        if let Some(target) = &self.target_path {
            if let Some(mut root) = self.hierarchy.take() {
                if let Some(node) = root.find_subtree_mut(target) {
                    let mut rng = Rng::with_seed(seed);
                    self.mutate_module_context_with_rng(&mut rng, &mut node.module);
                }
                self.hierarchy = Some(root);
                return;
            }
        }

        if let Some(mut root) = self.hierarchy.take() {
            let mut rng = Rng::with_seed(self.rng.u64(..));
            self.mutate_module_context_with_rng(&mut rng, &mut root.module);
            for child in &mut root.children {
                self.mutate_child_context_with_rng(&mut rng, child);
            }
            self.hierarchy = Some(root);
        }
    }

    fn mutate_child_context(&mut self, node: &mut HierarchyNode) {
        let mut rng = Rng::with_seed(self.rng.u64(..));
        self.mutate_module_context_with_rng(&mut rng, &mut node.module);
        for child in &mut node.children {
            self.mutate_child_context_with_rng(&mut rng, child);
        }
    }

    fn mutate_child_context_with_rng(&self, rng: &mut Rng, node: &mut HierarchyNode) {
        self.mutate_module_context_with_rng(rng, &mut node.module);
        for child in &mut node.children {
            self.mutate_child_context_with_rng(rng, child);
        }
    }

    fn mutate_module_context(&mut self, ctx: &mut ModuleContext) {
        let mut rng = Rng::with_seed(self.rng.u64(..));
        self.mutate_module_context_with_rng(&mut rng, ctx);
    }

    fn mutate_module_context_with_rng(&self, rng: &mut Rng, ctx: &mut ModuleContext) {
        if rng.f32() < 0.3 {
            self.mutate_signal_widths(ctx, rng);
        }
        if rng.f32() < 0.2 {
            self.mutate_signal_signedness(ctx, rng);
        }
        if rng.f32() < 0.2 {
            self.mutate_signal_kind(ctx, rng);
        }
        if rng.f32() < 0.15 {
            self.mutate_port_directions(ctx, rng);
        }
        if rng.f32() < 0.1 {
            self.mutate_generate_conditions(ctx, rng);
        }
        if rng.f32() < 0.1 {
            self.mutate_parameter_values(ctx, rng);
        }
    }

    fn mutate_signal_widths(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for signal in &mut ctx.signals {
            if rng.f32() < 0.4 {
                match &signal.width {
                    WidthKind::Fixed(w) => {
                        let boundaries = [1u32, 2, 4, 8, 16, 32, 64, 128, 256, 512];
                        let new_w = boundaries[rng.usize(0..boundaries.len())];
                        signal.width = WidthKind::Fixed(new_w);
                    }
                    WidthKind::Parameterized(name) => {
                        if rng.f32() < 0.5 {
                            signal.width = WidthKind::Expression(Expr::Var(
                                name.chars().next().unwrap_or('p'),
                            ));
                        }
                    }
                    WidthKind::Expression(_) => {}
                }
            }
        }
    }

    fn mutate_signal_signedness(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for signal in &mut ctx.signals {
            if rng.f32() < 0.3 {
                signal.is_signed = !signal.is_signed;
            }
        }
    }

    fn mutate_signal_kind(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for signal in &mut ctx.signals {
            if rng.f32() < 0.2 {
                signal.kind = match signal.kind {
                    SignalKind::Wire => SignalKind::Logic,
                    SignalKind::Logic => SignalKind::Reg,
                    SignalKind::Reg => SignalKind::Wire,
                    SignalKind::Input => SignalKind::Inout,
                    SignalKind::Output => SignalKind::Inout,
                    SignalKind::Inout => SignalKind::Wire,
                    SignalKind::Parameter => SignalKind::LocalParam,
                    SignalKind::LocalParam => SignalKind::Parameter,
                };
            }
        }
    }

    fn mutate_port_directions(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for signal in &mut ctx.signals {
            if rng.f32() < 0.3
                && matches!(
                    signal.kind,
                    SignalKind::Input | SignalKind::Output | SignalKind::Inout
                )
            {
                signal.kind = match signal.kind {
                    SignalKind::Input => SignalKind::Output,
                    SignalKind::Output => SignalKind::Inout,
                    SignalKind::Inout => SignalKind::Input,
                    _ => signal.kind,
                };
            }
        }
    }

    fn mutate_generate_conditions(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for gen_block in &mut ctx.generate_blocks {
            if rng.f32() < 0.4 {
                gen_block.condition = Some(self.gen_boundary_condition(rng));
            }
            if rng.f32() < 0.2 {
                gen_block.init = Some(Expr::Lit(rng.u64(0..1024)));
            }
        }
    }

    fn gen_boundary_condition(&self, rng: &mut Rng) -> Expr {
        let boundaries = [1, 2, 4, 8, 16, 31, 32, 33, 64, 128, 255, 256, 512, 1024];
        let bound = boundaries[rng.usize(0..boundaries.len())];
        let op_choices = [BinOp::Lt, BinOp::Gt, BinOp::Eq, BinOp::Ne];
        let op = op_choices[rng.usize(0..op_choices.len())];
        Expr::Bin(op, Box::new(Expr::Var('i')), Box::new(Expr::Lit(bound)))
    }

    fn mutate_parameter_values(&self, ctx: &mut ModuleContext, rng: &mut Rng) {
        for (_, value) in &mut ctx.parameters {
            if rng.f32() < 0.4 {
                *value = Expr::Lit(rng.u64(0..1024));
            }
        }
    }

    pub fn generate_targeted_testcase(&mut self, target_path: &str) -> Option<GenInput> {
        let node = self.hierarchy.as_ref()?.find_subtree(target_path)?;
        let module = &node.module;

        let w = module
            .signals
            .first()
            .map(|s| match &s.width {
                WidthKind::Fixed(w) => *w,
                _ => 16,
            })
            .unwrap_or(16);

        let mut rng = Rng::with_seed(self.rng.u64(..));
        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let a = rng.u64(..) & mask;
        let b = rng.u64(..) & mask;

        let expr = super::expr::gen_node(w, &mut rng, 0);

        let mut input = GenInput {
            w,
            wb: w,
            a,
            b,
            expr,
            seed: self.rng.u64(..),
        };
        input.normalize();
        Some(input)
    }
}

impl Default for HierarchyMutator {
    fn default() -> Self {
        Self::new(0)
    }
}
