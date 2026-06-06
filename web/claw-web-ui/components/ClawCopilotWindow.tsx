"use client";

/**
 * Unified agent dock: chat (left) + conversation list (right). Author: kejiqing
 */
import { useChatContext, type WindowProps } from "@copilotkit/react-ui";
import React, { useCallback, useEffect } from "react";
import { ClawChatPaneToolbar } from "./ClawChatPaneToolbar";
import { ClawConvRail } from "./ClawConvRail";

function isMacOS(): boolean {
  return typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
}

export function ClawCopilotWindow({
  children,
  clickOutsideToClose,
  shortcut,
  hitEscapeToClose,
}: WindowProps) {
  const windowRef = React.useRef<HTMLDivElement>(null);
  const { open, setOpen } = useChatContext();
  const chatNodes = React.Children.toArray(children).filter(Boolean);

  const handleClickOutside = useCallback(
    (event: MouseEvent) => {
      if (!clickOutsideToClose) return;
      const parentElement = windowRef.current?.parentElement;
      let className = "";
      if (event.target instanceof HTMLElement) {
        className = event.target.className;
      }
      if (
        open &&
        parentElement &&
        !parentElement.contains(event.target as Node) &&
        !className.includes("copilotKitDebugMenu")
      ) {
        setOpen(false);
      }
    },
    [clickOutsideToClose, open, setOpen],
  );

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      const isInput =
        target.tagName === "INPUT" ||
        target.tagName === "SELECT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable;
      const isDescendantOfWrapper = windowRef.current?.contains(target);

      if (
        open &&
        event.key === "Escape" &&
        (!isInput || isDescendantOfWrapper) &&
        hitEscapeToClose
      ) {
        setOpen(false);
      } else if (
        event.key === shortcut &&
        ((isMacOS() && event.metaKey) || (!isMacOS() && event.ctrlKey)) &&
        (!isInput || isDescendantOfWrapper)
      ) {
        setOpen(!open);
      }
    },
    [hitEscapeToClose, shortcut, open, setOpen],
  );

  const adjustForMobile = useCallback(() => {
    const el = windowRef.current;
    const vv = window.visualViewport;
    if (!el || !vv) return;

    if (window.innerWidth < 640 && open) {
      el.style.height = `${vv.height}px`;
      el.style.left = `${vv.offsetLeft}px`;
      el.style.top = `${vv.offsetTop}px`;
      document.body.style.position = "fixed";
      document.body.style.width = "100%";
      document.body.style.height = `${window.innerHeight}px`;
      document.body.style.overflow = "hidden";
      document.body.style.touchAction = "none";
      document.body.addEventListener("touchmove", preventScroll, { passive: false });
    } else {
      el.style.height = "";
      el.style.left = "";
      el.style.top = "";
      document.body.style.position = "";
      document.body.style.height = "";
      document.body.style.width = "";
      document.body.style.overflow = "";
      document.body.style.top = "";
      document.body.style.touchAction = "";
      document.body.removeEventListener("touchmove", preventScroll);
    }
  }, [open]);

  useEffect(() => {
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    if (window.visualViewport) {
      window.visualViewport.addEventListener("resize", adjustForMobile);
      adjustForMobile();
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener("resize", adjustForMobile);
      }
    };
  }, [adjustForMobile, handleClickOutside, handleKeyDown]);

  return (
    <div
      className={`copilotKitWindow claw-agent-dock${open ? " open" : ""}`}
      ref={windowRef}
    >
      <div className="claw-dock-layout">
        <div className="claw-dock-chat">
          <ClawChatPaneToolbar />
          <div className="claw-chat-body">{chatNodes}</div>
        </div>
        <ClawConvRail />
      </div>
    </div>
  );
}

function preventScroll(event: TouchEvent): void {
  let targetElement = event.target as Element;
  const hasParentWithClass = (element: Element, className: string): boolean => {
    while (element && element !== document.body) {
      if (element.classList.contains(className)) return true;
      element = element.parentElement!;
    }
    return false;
  };
  if (!hasParentWithClass(targetElement, "copilotKitMessages")) {
    event.preventDefault();
  }
}
