import type { CanvasConnection, CanvasNodeData, Position } from "@/types/canvas";

/** 层间水平间距（连线留白），对应 Tapnow 的 H_SPACING。 */
const H_SPACING = 150;
/** 层内垂直间距，对应 Tapnow 的 V_SPACING。 */
const V_SPACING = 40;
/** 层级计算的最大深度，防止环路导致无限松弛。 */
const MAX_LEVEL = 20;

type GraphNode = {
    id: string;
    node: CanvasNodeData;
    parents: string[];
    children: string[];
    level: number;
    barycenter?: number;
};

/**
 * 智能整理节点：DAG 层级布局 + 交叉最小化（Barycenter Heuristic）。
 * 算法移植自 Tapnow Studio 的“自动整理节点”，适配本项目节点模型：
 * 1. 依据连线在目标节点子集上构建有向图（只保留两端都在集合内的边）；
 * 2. 最长路径分层（BFS 松弛，根节点为第 0 层）；
 * 3. 层内按初始 y 排序后做 3 轮正/反向重心扫描，减少连线交叉；
 * 4. 逐层从左到右排布：层内节点垂直堆叠，层间水平前进（按该层最大宽度）。
 * 纯函数，不修改输入；返回每个目标节点的新坐标。
 */
export function autoArrangeNodes(targets: CanvasNodeData[], connections: CanvasConnection[]): Map<string, Position> {
    const ids = new Set(targets.map((node) => node.id));
    const graph: Record<string, GraphNode> = {};
    targets.forEach((node) => {
        graph[node.id] = { id: node.id, node, parents: [], children: [], level: 0 };
    });

    connections.forEach((connection) => {
        if (ids.has(connection.fromNodeId) && ids.has(connection.toNodeId)) {
            graph[connection.fromNodeId].children.push(connection.toNodeId);
            graph[connection.toNodeId].parents.push(connection.fromNodeId);
        }
    });

    // 2. 计算层级（最长路径）：入度为 0 的节点为根；全成环时退化为取第一个节点。
    let roots = Object.values(graph).filter((node) => node.parents.length === 0);
    if (roots.length === 0 && targets.length > 0) roots = [Object.values(graph)[0]];
    const queue = roots.map((node) => ({ node, lvl: 0 }));
    while (queue.length > 0) {
        const { node, lvl } = queue.shift()!;
        if (lvl >= node.level) {
            node.level = lvl;
            if (lvl < MAX_LEVEL) {
                node.children.forEach((childId) => {
                    const child = graph[childId];
                    if (child) queue.push({ node: child, lvl: lvl + 1 });
                });
            }
        }
    }

    // 3. 构建层级数组，初始按当前 y 排序保持原有阅读顺序。
    const maxLevel = Math.max(...Object.values(graph).map((node) => node.level));
    const layers: GraphNode[][] = Array.from({ length: maxLevel + 1 }, () => []);
    Object.values(graph).forEach((node) => layers[node.level].push(node));
    layers.forEach((layer) => layer.sort((a, b) => a.node.position.y - b.node.position.y));

    // 4. 交叉最小化：3 轮正/反向重心扫描。
    for (let i = 0; i < 3; i += 1) {
        for (let l = 1; l < layers.length; l += 1) {
            const layer = layers[l];
            layer.forEach((node) => {
                if (node.parents.length > 0) {
                    let sumRank = 0;
                    node.parents.forEach((parentId) => {
                        const rank = layers[l - 1].indexOf(graph[parentId]);
                        if (rank !== -1) sumRank += rank;
                    });
                    node.barycenter = sumRank / node.parents.length;
                } else {
                    node.barycenter = layer.indexOf(node);
                }
            });
            layer.sort((a, b) => (a.barycenter ?? 0) - (b.barycenter ?? 0));
        }
        for (let l = layers.length - 2; l >= 0; l -= 1) {
            const layer = layers[l];
            layer.forEach((node) => {
                if (node.children.length > 0) {
                    let sumRank = 0;
                    node.children.forEach((childId) => {
                        const rank = layers[l + 1].indexOf(graph[childId]);
                        if (rank !== -1) sumRank += rank;
                    });
                    node.barycenter = sumRank / node.children.length;
                } else {
                    node.barycenter = layer.indexOf(node);
                }
            });
            layer.sort((a, b) => (a.barycenter ?? 0) - (b.barycenter ?? 0));
        }
    }

    // 5. 坐标分配：层从左到右（x = 上一层最大宽度 + H_SPACING），层内自上而下垂直堆叠。
    const startX = Math.min(...targets.map((node) => node.position.x));
    const startY = Math.min(...targets.map((node) => node.position.y));
    const positions = new Map<string, Position>();
    let currentX = startX;
    layers.forEach((layer) => {
        if (layer.length === 0) return;
        const maxW = Math.max(...layer.map((node) => node.node.width));
        let currentY = startY;
        layer.forEach((graphNode) => {
            positions.set(graphNode.id, { x: currentX, y: currentY });
            currentY += graphNode.node.height + V_SPACING;
        });
        currentX += maxW + H_SPACING;
    });
    return positions;
}
