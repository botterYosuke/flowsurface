"""Tachibana Securities order/holdings endpoints."""

from __future__ import annotations

from ._client import Client


class Tachibana:
    def __init__(self, client: Client) -> None:
        self._c = client

    @property
    def buying_power(self) -> object:
        """GET /api/buying-power"""
        return self._c.get("/api/buying-power")

    def orders(self, eig_day: str | None = None) -> object:
        """GET /api/tachibana/orders[?eig_day=YYYYMMDD]"""
        kwargs = {"eig_day": eig_day} if eig_day else {}
        return self._c.get("/api/tachibana/orders", **kwargs)

    def order_detail(self, order_num: str, eig_day: str | None = None) -> object:
        """GET /api/tachibana/order/{order_num}[?eig_day=YYYYMMDD]"""
        path = f"/api/tachibana/order/{order_num}"
        kwargs = {"eig_day": eig_day} if eig_day else {}
        return self._c.get(path, **kwargs)

    def holdings(self, issue_code: str) -> object:
        """GET /api/tachibana/holdings?issue_code=…"""
        return self._c.get("/api/tachibana/holdings", issue_code=issue_code)

    def new_order(
        self,
        issue_code: str,
        qty: str,
        side: str,
        price: str,
        second_password: str,
        *,
        account_type: str | None = None,
        market_code: str | None = None,
        condition: str | None = None,
        cash_margin: str | None = None,
        expire_day: str | None = None,
    ) -> object:
        """POST /api/tachibana/order — place a real Tachibana order.

        Args:
            issue_code:      e.g. "7203"
            qty:             e.g. "100"
            side:            "buy" or "sell"
            price:           limit price string; "0" for market order
            second_password: required for order submission
        """
        return self._c.post(
            "/api/tachibana/order",
            {
                "issue_code": issue_code,
                "qty": qty,
                "side": side,
                "price": price,
                "second_password": second_password,
                "account_type": account_type,
                "market_code": market_code,
                "condition": condition,
                "cash_margin": cash_margin,
                "expire_day": expire_day,
            },
        )

    def correct_order(
        self,
        order_number: str,
        eig_day: str,
        second_password: str,
        *,
        condition: str | None = None,
        price: str | None = None,
        qty: str | None = None,
        expire_day: str | None = None,
    ) -> object:
        """POST /api/tachibana/order/correct"""
        return self._c.post(
            "/api/tachibana/order/correct",
            {
                "order_number": order_number,
                "eig_day": eig_day,
                "second_password": second_password,
                "condition": condition,
                "price": price,
                "qty": qty,
                "expire_day": expire_day,
            },
        )

    def cancel_order(
        self,
        order_number: str,
        eig_day: str,
        second_password: str,
    ) -> object:
        """POST /api/tachibana/order/cancel"""
        return self._c.post(
            "/api/tachibana/order/cancel",
            {
                "order_number": order_number,
                "eig_day": eig_day,
                "second_password": second_password,
            },
        )
