/*
  Copyright 2017 Takashi Ogura

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
*/

#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use kdtree::distance::squared_euclidean;
use num_traits::float::Float;
use num_traits::identities::Zero;
use rand::distributions::{Distribution, Uniform};
use std::fmt::Debug;
use std::mem;
use tracing::debug;

#[derive(Debug)]
enum ExtendStatus {
    Reached(usize),
    Advanced(usize),
    Trapped,
}

/// Node that contains user data
#[derive(Debug, Clone)]
struct Node<T> {
    parent_index: Option<usize>,
    data: T,
}

impl<T> Node<T> {
    fn new(data: T) -> Self {
        Node {
            parent_index: None,
            data,
        }
    }
}

/// RRT
#[derive(Debug)]
struct Tree<N>
where
    N: Float + Zero + Debug,
{
    kdtree: kdtree::KdTree<N, usize, Vec<N>>,
    vertices: Vec<Node<Vec<N>>>,
    name: &'static str,
}

impl<N> Tree<N>
where
    N: Float + Zero + Debug,
{
    /// Create a new tree
    fn new(name: &'static str, dim: usize) -> Self {
        Tree {
            kdtree: kdtree::KdTree::new(dim),
            vertices: Vec::new(),
            name,
        }
    }

    /// Add a vertex to the tree
    fn add_vertex(&mut self, q: &[N]) -> usize {
        let index = self.vertices.len();
        self.kdtree.add(q.to_vec(), index).unwrap();
        self.vertices.push(Node::new(q.to_vec()));
        index
    }

    /// Add an edge between two vertices
    fn add_edge(&mut self, q1_index: usize, q2_index: usize) {
        self.vertices[q2_index].parent_index = Some(q1_index);
    }

    /// Get the nearest index from the tree
    fn get_nearest_index(&self, q: &[N]) -> usize {
        *self.kdtree.nearest(q, 1, &squared_euclidean).unwrap()[0].1
    }

    /// RRT* Extension: Get the nearest indicex in a radius
    fn get_nearest_indices_in_radius(&self, q: &[N], radius: N) -> Vec<usize> {
        self.kdtree
            .within(q, radius, &squared_euclidean)
            .unwrap_or(vec![])
            .into_iter()
            .map(|(_, i)| *i)
            .collect::<Vec<usize>>()
    }

    /// RRT* Extension: Either extend this extend function to optionally reqire or make an extend_rewire
    /// Extend the tree to the target point
    fn extend<FF>(&mut self, q_target: &[N], extend_length: N, is_free: &mut FF) -> ExtendStatus
    where
        FF: FnMut(&[N]) -> bool,
    {
        assert!(extend_length > N::zero());
        let nearest_index = self.get_nearest_index(q_target);
        let nearest_q = &self.vertices[nearest_index].data;
        let diff_dist = squared_euclidean(q_target, nearest_q).sqrt();
        let q_new = if diff_dist < extend_length {
            q_target.to_vec()
        } else {
            nearest_q
                .iter()
                .zip(q_target)
                .map(|(near, target)| *near + (*target - *near) * extend_length / diff_dist)
                .collect::<Vec<_>>()
        };
        debug!("q_new={q_new:?}");
        if is_free(&q_new) {
            let new_index = self.add_vertex(&q_new);
            self.add_edge(nearest_index, new_index);
            if squared_euclidean(&q_new, q_target).sqrt() < extend_length {
                return ExtendStatus::Reached(new_index);
            }
            debug!("target = {q_target:?}");
            debug!("advanced to {q_target:?}");
            return ExtendStatus::Advanced(new_index);
        }
        ExtendStatus::Trapped
    }

    /// RRT* Extend Function with Rewiring
    fn extend_rewire<FF>(
        &mut self,
        q_target: &[N],
        extend_length: N,
        is_free: &mut FF,
    ) -> ExtendStatus
    where
        FF: FnMut(&[N]) -> bool,
    {
        assert!(extend_length > N::zero());
        let nearest_index = self.get_nearest_index(q_target);
        let nearest_q = &self.vertices[nearest_index].data;
        let diff_dist = squared_euclidean(q_target, nearest_q).sqrt();
        let q_new = if diff_dist < extend_length {
            q_target.to_vec()
        } else {
            nearest_q
                .iter()
                .zip(q_target)
                .map(|(near, target)| *near + (*target - *near) * extend_length / diff_dist)
                .collect::<Vec<_>>()
        };
        if is_free(&q_new) {
            let new_index = self.add_vertex(&q_new);
            self.add_edge(nearest_index, new_index);

            // Rewiring process
            let neighbors = self.get_nearest_indices_in_radius(&q_new, extend_length);
            // Update parent if the new point is closer to the neighbor
            for &neighbor_index in &neighbors {
                // Skip if the neighbor doesn't have a parent
                if let Some(parent_index) = self.vertices[neighbor_index].parent_index {
                    let neighbor_q = &self.vertices[neighbor_index].data;
                    // Update parent if the new point is closer to the neighbor
                    if squared_euclidean(&q_new, neighbor_q)
                        < squared_euclidean(&self.vertices[parent_index].data, neighbor_q)
                    {
                        self.vertices[neighbor_index].parent_index = Some(new_index);
                    }
                }
            }

            if squared_euclidean(&q_new, q_target) < extend_length {
                return ExtendStatus::Reached(new_index);
            }
            return ExtendStatus::Advanced(new_index);
        }
        ExtendStatus::Trapped
    }

    /// Connect the tree to the target point
    fn connect<FF>(&mut self, q_target: &[N], extend_length: N, is_free: &mut FF) -> ExtendStatus
    where
        FF: FnMut(&[N]) -> bool,
    {
        loop {
            debug!("connecting...{q_target:?}");
            match self.extend(q_target, extend_length, is_free) {
                ExtendStatus::Trapped => return ExtendStatus::Trapped,
                ExtendStatus::Reached(index) => return ExtendStatus::Reached(index),
                ExtendStatus::Advanced(_) => {}
            };
        }
    }

    /// Get all nodes from leaf to the root
    fn get_until_root(&self, index: usize) -> Vec<Vec<N>> {
        let mut nodes = Vec::new();
        let mut cur_index = index;
        while let Some(parent_index) = self.vertices[cur_index].parent_index {
            cur_index = parent_index;
            nodes.push(self.vertices[cur_index].data.clone())
        }
        nodes
    }
}

/// RRT* Extension: connect with RRT* algorithm
pub fn rrt_star_connect<FF, FR, N>(
    start: &[N],
    goal: &[N],
    mut is_free: FF,
    random_sample: FR,
    extend_length: N,
    num_max_try: usize,
) -> Result<Vec<Vec<N>>, String>
where
    FF: FnMut(&[N]) -> bool,
    FR: Fn() -> Vec<N>,
    N: Float + Debug,
{
    let mut tree = Tree::new("rrt_star", start.len());
    tree.add_vertex(start);

    let mut closest_to_goal = start.to_vec();
    let mut min_dist_to_goal = squared_euclidean(goal, start).sqrt();

    for _ in 0..num_max_try {
        let q_rand = if rand::random::<f64>() < 0.1 {
            // Bias towards goal with 10% probability
            goal.to_vec()
        } else {
            random_sample()
        };

        match tree.extend_rewire(&q_rand, extend_length, &mut is_free) {
            ExtendStatus::Trapped => continue,
            ExtendStatus::Advanced(index) | ExtendStatus::Reached(index) => {
                let new_point = &tree.vertices[index].data;
                let dist_to_goal = squared_euclidean(goal, new_point).sqrt();
                if dist_to_goal < min_dist_to_goal {
                    closest_to_goal = new_point.clone();
                    min_dist_to_goal = dist_to_goal;
                }

                // Try to connect directly to goal if close enough
                if dist_to_goal < extend_length && is_free(goal) {
                    tree.add_vertex(goal);
                    tree.add_edge(index, tree.vertices.len() - 1);
                    return Ok(tree.get_until_root(tree.vertices.len() - 1));
                }
            }
        }
    }

    // If no direct connection to the goal is possible, return the path to the closest point
    let index_of_closest = tree
        .vertices
        .iter()
        .position(|v| v.data == closest_to_goal)
        .unwrap();
    Ok(tree.get_until_root(index_of_closest))
}

/// search the path from start to goal which is free, using random_sample function
pub fn dual_rrt_connect<FF, FR, N>(
    start: &[N],
    goal: &[N],
    mut is_free: FF,
    random_sample: FR,
    extend_length: N,
    num_max_try: usize,
) -> Result<Vec<Vec<N>>, String>
where
    FF: FnMut(&[N]) -> bool,
    FR: Fn() -> Vec<N>,
    N: Float + Debug,
{
    assert_eq!(start.len(), goal.len());
    let mut tree_a = Tree::new("start", start.len());
    let mut tree_b = Tree::new("goal", start.len());
    tree_a.add_vertex(start);
    tree_b.add_vertex(goal);
    for _ in 0..num_max_try {
        debug!("tree_a = {:?}", tree_a.vertices.len());
        debug!("tree_b = {:?}", tree_b.vertices.len());
        let q_rand = random_sample();
        let extend_status = tree_a.extend(&q_rand, extend_length, &mut is_free);
        match extend_status {
            ExtendStatus::Trapped => {}
            ExtendStatus::Advanced(new_index) | ExtendStatus::Reached(new_index) => {
                let q_new = &tree_a.vertices[new_index].data;
                if let ExtendStatus::Reached(reach_index) =
                    tree_b.connect(q_new, extend_length, &mut is_free)
                {
                    let mut a_all = tree_a.get_until_root(new_index);
                    let mut b_all = tree_b.get_until_root(reach_index);
                    a_all.reverse();
                    a_all.append(&mut b_all);
                    if tree_b.name == "start" {
                        a_all.reverse();
                    }
                    return Ok(a_all);
                }
            }
        }
        mem::swap(&mut tree_a, &mut tree_b);
    }
    Err("failed".to_string())
}

/// select random two points, and try to connect.
pub fn smooth_path<FF, N>(
    path: &mut Vec<Vec<N>>,
    mut is_free: FF,
    extend_length: N,
    num_max_try: usize,
) where
    FF: FnMut(&[N]) -> bool,
    N: Float + Debug,
{
    if path.len() < 3 {
        return;
    }
    let mut rng = rand::thread_rng();
    for _ in 0..num_max_try {
        let range1 = Uniform::new(0, path.len() - 2);
        let ind1 = range1.sample(&mut rng);
        let range2 = Uniform::new(ind1 + 2, path.len());
        let ind2 = range2.sample(&mut rng);
        let mut base_point = path[ind1].clone();
        let point2 = path[ind2].clone();
        let mut is_searching = true;
        while is_searching {
            let diff_dist = squared_euclidean(&base_point, &point2).sqrt();
            if diff_dist < extend_length {
                // reached!
                // remove path[ind1+1] ... path[ind2-1]
                let remove_index = ind1 + 1;
                for _ in 0..(ind2 - ind1 - 1) {
                    path.remove(remove_index);
                }
                if path.len() == 2 {
                    return;
                }
                is_searching = false;
            } else {
                let check_point = base_point
                    .iter()
                    .zip(point2.iter())
                    .map(|(near, target)| *near + (*target - *near) * extend_length / diff_dist)
                    .collect::<Vec<_>>();
                if !is_free(&check_point) {
                    // trapped
                    is_searching = false;
                } else {
                    // continue to extend
                    base_point = check_point;
                }
            }
        }
    }
}

#[test]
fn it_works() {
    use rand::distributions::{Distribution, Uniform};
    let mut result = dual_rrt_connect(
        &[-1.2, 0.0],
        &[1.2, 0.0],
        |p: &[f64]| !(p[0].abs() < 1.0 && p[1].abs() < 1.0),
        || {
            let between = Uniform::new(-2.0, 2.0);
            let mut rng = rand::thread_rng();
            vec![between.sample(&mut rng), between.sample(&mut rng)]
        },
        0.2,
        1000,
    )
    .unwrap();
    println!("{result:?}");
    assert!(result.len() >= 4);
    smooth_path(
        &mut result,
        |p: &[f64]| !(p[0].abs() < 1.0 && p[1].abs() < 1.0),
        0.2,
        100,
    );
    println!("{result:?}");
    assert!(result.len() >= 3);
}
