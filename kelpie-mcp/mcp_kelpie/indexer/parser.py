"""
Tree-sitter based Rust parser for indexing.

Uses tree-sitter-rust to parse Rust source files and extract symbols.
"""

from pathlib import Path
from typing import Iterator

import tree_sitter_rust as ts_rust
from tree_sitter import Language, Parser, Node

from .types import (
    Symbol,
    SymbolKind,
    Visibility,
    Import,
    FileSymbols,
)


class RustParser:
    """Parse Rust source files using tree-sitter."""

    def __init__(self):
        """Initialize the parser with Rust language."""
        self.parser = Parser(Language(ts_rust.language()))

    def parse_file(self, path: Path) -> FileSymbols | None:
        """Parse a Rust source file and extract symbols.

        Args:
            path: Path to the .rs file

        Returns:
            FileSymbols or None if parsing fails
        """
        try:
            content = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            return None

        return self.parse_content(str(path), content)

    def parse_content(self, path: str, content: str) -> FileSymbols:
        """Parse Rust source content and extract symbols.

        Args:
            path: File path (for reference)
            content: Rust source code

        Returns:
            FileSymbols with extracted symbols and imports
        """
        tree = self.parser.parse(content.encode("utf-8"))
        root = tree.root_node

        symbols: list[Symbol] = []
        imports: list[Import] = []

        # Walk the tree and extract symbols
        for node, attrs in self._walk_tree(root):
            if node.type == "function_item":
                symbol = self._parse_function(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "struct_item":
                symbol = self._parse_struct(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "enum_item":
                symbol = self._parse_enum(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "trait_item":
                symbol = self._parse_trait(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "impl_item":
                symbol = self._parse_impl(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "const_item":
                symbol = self._parse_const(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "static_item":
                symbol = self._parse_static(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "type_item":
                symbol = self._parse_type_alias(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "macro_definition":
                symbol = self._parse_macro(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "mod_item":
                symbol = self._parse_mod(node, content, attrs)
                if symbol:
                    symbols.append(symbol)
            elif node.type == "use_declaration":
                import_info = self._parse_use(node, content)
                if import_info:
                    imports.extend(import_info)

        return FileSymbols(path=path, symbols=symbols, imports=imports)

    def _walk_tree(self, node: Node) -> Iterator[tuple[Node, list[Node]]]:
        """Walk tree yielding top-level items with their preceding attributes.

        Args:
            node: Root node to walk

        Yields:
            Tuples of (item_node, preceding_attributes)
        """
        pending_attrs: list[Node] = []

        for child in node.children:
            if child.type == "attribute_item":
                # Collect attributes for the next item
                pending_attrs.append(child)
            else:
                yield (child, pending_attrs)
                pending_attrs = []

                # Don't recurse into function bodies, but do recurse into impl blocks
                if child.type in ("impl_item", "mod_item"):
                    for grandchild in child.children:
                        if grandchild.type == "declaration_list":
                            inner_attrs: list[Node] = []
                            for item in grandchild.children:
                                if item.type == "attribute_item":
                                    inner_attrs.append(item)
                                else:
                                    yield (item, inner_attrs)
                                    inner_attrs = []

    def _get_visibility(self, node: Node) -> Visibility:
        """Extract visibility from a node.

        Args:
            node: Item node

        Returns:
            Visibility enum value
        """
        for child in node.children:
            if child.type == "visibility_modifier":
                text = child.text.decode("utf-8") if child.text else ""
                if text == "pub":
                    return Visibility.PUBLIC
                elif "crate" in text:
                    return Visibility.CRATE
                elif "super" in text:
                    return Visibility.SUPER
        return Visibility.PRIVATE

    def _get_doc_comment(self, node: Node, content: str) -> str | None:
        """Extract doc comment preceding a node.

        Args:
            node: Item node
            content: Full source content

        Returns:
            Doc comment text or None
        """
        # Look at preceding siblings for doc comments
        lines = content.split("\n")
        start_line = node.start_point[0]

        if start_line == 0:
            return None

        doc_lines: list[str] = []
        line_idx = start_line - 1

        while line_idx >= 0:
            line = lines[line_idx].strip()
            if line.startswith("///"):
                doc_lines.insert(0, line[3:].strip())
                line_idx -= 1
            elif line.startswith("//!"):
                # Module doc comment, stop
                break
            elif line == "" or line.startswith("//"):
                line_idx -= 1
            else:
                break

        return "\n".join(doc_lines) if doc_lines else None

    def _get_attributes(self, node: Node, external_attrs: list[Node] | None = None) -> list[str]:
        """Extract attributes from a node and external attribute nodes.

        Args:
            node: Item node
            external_attrs: List of preceding attribute_item nodes

        Returns:
            List of attribute strings
        """
        attrs: list[str] = []

        # First add external (preceding) attributes
        if external_attrs:
            for attr_node in external_attrs:
                text = attr_node.text.decode("utf-8") if attr_node.text else ""
                # Remove the #[ and ]
                if text.startswith("#[") and text.endswith("]"):
                    attrs.append(text[2:-1])

        # Then add inline attributes (children of the node)
        for child in node.children:
            if child.type == "attribute_item":
                text = child.text.decode("utf-8") if child.text else ""
                # Remove the #[ and ]
                if text.startswith("#[") and text.endswith("]"):
                    attrs.append(text[2:-1])
        return attrs

    def _find_child_by_type(self, node: Node, type_name: str) -> Node | None:
        """Find first child with given type.

        Args:
            node: Parent node
            type_name: Type to find

        Returns:
            Child node or None
        """
        for child in node.children:
            if child.type == type_name:
                return child
        return None

    def _find_child_by_field(self, node: Node, field_name: str) -> Node | None:
        """Find child by field name.

        Args:
            node: Parent node
            field_name: Field name to find

        Returns:
            Child node or None
        """
        return node.child_by_field_name(field_name)

    def _parse_function(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a function item.

        Args:
            node: function_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        # Check for async and unsafe in function_modifiers
        is_async = False
        is_unsafe = False
        for child in node.children:
            if child.type == "function_modifiers":
                for mod in child.children:
                    if mod.type == "async":
                        is_async = True
                    elif mod.type == "unsafe":
                        is_unsafe = True
            elif child.type == "async":
                is_async = True
            elif child.type == "unsafe":
                is_unsafe = True

        # Get attributes
        attrs = self._get_attributes(node, external_attrs)

        # Check if test
        is_test = any("test" in a for a in attrs)

        # Get generic params
        generic_params: list[str] = []
        type_params = self._find_child_by_type(node, "type_parameters")
        if type_params:
            for child in type_params.children:
                if child.type == "type_parameter":
                    # type_parameter contains type_identifier
                    for subchild in child.children:
                        if subchild.type == "type_identifier":
                            generic_params.append(subchild.text.decode("utf-8") if subchild.text else "")
                elif child.type == "type_identifier":
                    generic_params.append(child.text.decode("utf-8") if child.text else "")

        # Build signature
        params_node = self._find_child_by_type(node, "parameters")
        return_type = self._find_child_by_type(node, "return_type")

        sig_parts = []
        if is_async:
            sig_parts.append("async ")
        if is_unsafe:
            sig_parts.append("unsafe ")
        sig_parts.append(f"fn {name}")
        if generic_params:
            sig_parts.append(f"<{', '.join(generic_params)}>")
        if params_node:
            sig_parts.append(params_node.text.decode("utf-8") if params_node.text else "()")
        if return_type:
            sig_parts.append(f" {return_type.text.decode('utf-8') if return_type.text else ''}")

        return Symbol(
            name=name,
            kind=SymbolKind.FUNCTION,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            signature="".join(sig_parts),
            doc=self._get_doc_comment(node, content),
            is_async=is_async,
            is_test=is_test,
            is_unsafe=is_unsafe,
            generic_params=generic_params,
            attributes=attrs,
        )

    def _parse_struct(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a struct item.

        Args:
            node: struct_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        generic_params: list[str] = []
        type_params = self._find_child_by_type(node, "type_parameters")
        if type_params:
            for child in type_params.children:
                if child.type == "type_parameter":
                    for subchild in child.children:
                        if subchild.type == "type_identifier":
                            generic_params.append(subchild.text.decode("utf-8") if subchild.text else "")
                elif child.type == "type_identifier":
                    generic_params.append(child.text.decode("utf-8") if child.text else "")

        return Symbol(
            name=name,
            kind=SymbolKind.STRUCT,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            doc=self._get_doc_comment(node, content),
            generic_params=generic_params,
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_enum(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse an enum item.

        Args:
            node: enum_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        generic_params: list[str] = []
        type_params = self._find_child_by_type(node, "type_parameters")
        if type_params:
            for child in type_params.children:
                if child.type == "type_parameter":
                    for subchild in child.children:
                        if subchild.type == "type_identifier":
                            generic_params.append(subchild.text.decode("utf-8") if subchild.text else "")
                elif child.type == "type_identifier":
                    generic_params.append(child.text.decode("utf-8") if child.text else "")

        return Symbol(
            name=name,
            kind=SymbolKind.ENUM,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            doc=self._get_doc_comment(node, content),
            generic_params=generic_params,
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_trait(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a trait item.

        Args:
            node: trait_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        generic_params: list[str] = []
        type_params = self._find_child_by_type(node, "type_parameters")
        if type_params:
            for child in type_params.children:
                if child.type == "type_parameter":
                    for subchild in child.children:
                        if subchild.type == "type_identifier":
                            generic_params.append(subchild.text.decode("utf-8") if subchild.text else "")
                elif child.type == "type_identifier":
                    generic_params.append(child.text.decode("utf-8") if child.text else "")

        return Symbol(
            name=name,
            kind=SymbolKind.TRAIT,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            doc=self._get_doc_comment(node, content),
            generic_params=generic_params,
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_impl(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse an impl item.

        Args:
            node: impl_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        # Get the type being implemented
        type_node = self._find_child_by_field(node, "type")
        if not type_node:
            return None

        type_text = type_node.text.decode("utf-8") if type_node.text else ""

        # Check for trait implementation
        trait_node = self._find_child_by_field(node, "trait")
        if trait_node:
            trait_text = trait_node.text.decode("utf-8") if trait_node.text else ""
            name = f"{trait_text} for {type_text}"
        else:
            name = type_text

        return Symbol(
            name=name,
            kind=SymbolKind.IMPL,
            line=node.start_point[0] + 1,
            visibility=Visibility.PRIVATE,  # impls don't have visibility
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_const(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a const item.

        Args:
            node: const_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        # Get type
        type_node = self._find_child_by_field(node, "type")
        type_text = type_node.text.decode("utf-8") if type_node and type_node.text else ""

        return Symbol(
            name=name,
            kind=SymbolKind.CONST,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            signature=f"const {name}: {type_text}" if type_text else f"const {name}",
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_static(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a static item.

        Args:
            node: static_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        # Check for mutable
        is_mut = any(c.type == "mutable_specifier" for c in node.children)

        # Get type
        type_node = self._find_child_by_field(node, "type")
        type_text = type_node.text.decode("utf-8") if type_node and type_node.text else ""

        mut_str = "mut " if is_mut else ""
        return Symbol(
            name=name,
            kind=SymbolKind.STATIC,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            signature=f"static {mut_str}{name}: {type_text}" if type_text else f"static {mut_str}{name}",
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_type_alias(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a type alias.

        Args:
            node: type_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        return Symbol(
            name=name,
            kind=SymbolKind.TYPE_ALIAS,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_macro(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a macro definition.

        Args:
            node: macro_definition node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        return Symbol(
            name=name,
            kind=SymbolKind.MACRO,
            line=node.start_point[0] + 1,
            visibility=Visibility.PUBLIC,  # macros are exported
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_mod(self, node: Node, content: str, external_attrs: list[Node] | None = None) -> Symbol | None:
        """Parse a mod item.

        Args:
            node: mod_item node
            content: Source content
            external_attrs: Preceding attribute nodes

        Returns:
            Symbol or None
        """
        name_node = self._find_child_by_field(node, "name")
        if not name_node:
            return None

        name = name_node.text.decode("utf-8") if name_node.text else ""

        return Symbol(
            name=name,
            kind=SymbolKind.MOD,
            line=node.start_point[0] + 1,
            visibility=self._get_visibility(node),
            doc=self._get_doc_comment(node, content),
            attributes=self._get_attributes(node, external_attrs),
        )

    def _parse_use(self, node: Node, content: str) -> list[Import]:
        """Parse a use declaration.

        Args:
            node: use_declaration node
            content: Source content

        Returns:
            List of imports
        """
        imports: list[Import] = []

        # Get the use argument
        for child in node.children:
            if child.type in ("use_as_clause", "scoped_use_list", "use_wildcard", "scoped_identifier", "identifier"):
                imports.extend(self._extract_imports(child))

        return imports

    def _extract_imports(self, node: Node, prefix: str = "") -> list[Import]:
        """Recursively extract imports from use tree.

        Args:
            node: Use tree node
            prefix: Path prefix

        Returns:
            List of imports
        """
        imports: list[Import] = []

        if node.type == "use_wildcard":
            # use foo::*
            path_node = node.children[0] if node.children else None
            if path_node:
                path = path_node.text.decode("utf-8") if path_node.text else ""
                imports.append(Import(path=f"{prefix}{path}::*", is_glob=True))
            else:
                imports.append(Import(path=f"{prefix}*", is_glob=True))
        elif node.type == "use_as_clause":
            # use foo as bar
            path_node = node.children[0] if node.children else None
            alias_node = node.children[-1] if len(node.children) > 2 else None
            if path_node:
                path = path_node.text.decode("utf-8") if path_node.text else ""
                alias = alias_node.text.decode("utf-8") if alias_node and alias_node.text else None
                imports.append(Import(path=f"{prefix}{path}", alias=alias))
        elif node.type == "scoped_use_list":
            # use foo::{bar, baz}
            path_parts = []
            for child in node.children:
                if child.type in ("identifier", "scoped_identifier"):
                    path_parts.append(child.text.decode("utf-8") if child.text else "")
                elif child.type == "use_list":
                    new_prefix = "::".join(path_parts) + "::" if path_parts else prefix
                    for list_child in child.children:
                        imports.extend(self._extract_imports(list_child, new_prefix))
        elif node.type == "scoped_identifier":
            # use foo::bar
            path = node.text.decode("utf-8") if node.text else ""
            imports.append(Import(path=f"{prefix}{path}"))
        elif node.type == "identifier":
            # use foo
            name = node.text.decode("utf-8") if node.text else ""
            imports.append(Import(path=f"{prefix}{name}"))

        return imports
