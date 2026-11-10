import { useMemo, useState } from "react";

function Tag({ element, closing = false, empty = false }: { element: Element; closing?: boolean; empty?: boolean }) {
  return <><span className="xml-punctuation">{closing ? "</" : "<"}</span><span className="xml-tag">{element.tagName}</span>{!closing && Array.from(element.attributes).map((attribute) => <span key={attribute.name}> <span className="xml-attribute">{attribute.name}</span>=<span className="xml-string">{JSON.stringify(attribute.value)}</span></span>)}<span className="xml-punctuation">{empty ? " />" : ">"}</span></>;
}

function XmlNode({ node }: { node: Node }) {
  const [expanded, setExpanded] = useState(true);
  if (node.nodeType !== Node.ELEMENT_NODE) {
    const serialized = new XMLSerializer().serializeToString(node);
    return <div className={`xml-line ${node.nodeType === Node.COMMENT_NODE ? "xml-comment" : "xml-text"}`}>{serialized}</div>;
  }
  const element = node as Element;
  const children = Array.from(node.childNodes).filter((child) => child.nodeType !== Node.TEXT_NODE || child.textContent?.trim());
  if (!children.length) return <div className="xml-line"><Tag element={element} empty /></div>;
  return <div className="xml-node">
    <div className="xml-line"><button className="xml-toggle" aria-label={`${expanded ? "Collapse" : "Expand"} ${element.tagName}`} aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? "-" : "+"}</button><Tag element={element} />{!expanded && <><span className="xml-comment"> ... </span><Tag element={element} closing /></>}</div>
    {expanded && <><div className="xml-children">{children.map((child, index) => <XmlNode key={index} node={child} />)}</div><div className="xml-line"><Tag element={element} closing /></div></>}
  </div>;
}

export function XmlViewer({ text }: { text: string }) {
  const document = useMemo(() => {
    if (!text.trimStart().startsWith("<")) return null;
    const parsed = new DOMParser().parseFromString(text, "application/xml");
    return parsed.querySelector("parsererror") ? null : parsed;
  }, [text]);
  return <div className="xml-viewer" aria-label="Store value">
    {document ? Array.from(document.childNodes).map((node, index) => <XmlNode key={index} node={node} />) : <pre className="raw-value">{text}</pre>}
  </div>;
}
