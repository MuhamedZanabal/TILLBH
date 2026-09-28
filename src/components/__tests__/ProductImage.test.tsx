import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const images = vi.fn<(hashes: string[]) => Promise<Record<string, string>>>();
vi.mock("../../api", () => ({ api: { products: { images: (h: string[]) => images(h) } } }));

import { ProductImage, productMonogram } from "../ProductImage";

const H = (c: string) => c.repeat(64);
const PNG = "data:image/jpeg;base64,AAAA";

beforeEach(() => {
  images.mockReset();
});

describe("ProductImage", () => {
  it("shows a monogram placeholder without asking the server when there is no image", () => {
    render(<ProductImage name="Almarai Fresh Milk" />);
    const el = screen.getByTestId("product-image");
    expect(el.dataset.state).toBe("placeholder");
    expect(el.textContent).toBe("AF");
    expect(images).not.toHaveBeenCalled();
  });

  it("loads a stored image by hash and batches many hashes into one call", async () => {
    images.mockImplementation(async (hs) => Object.fromEntries(hs.map((h) => [h, PNG])));
    render(
      <>
        <ProductImage hash={H("a")} name="One" />
        <ProductImage hash={H("b")} name="Two" />
        <ProductImage hash={H("a")} name="One again" />
      </>,
    );
    await waitFor(() => expect(screen.getAllByRole("img")).toHaveLength(3));
    expect(images).toHaveBeenCalledTimes(1);
    expect(images.mock.calls[0][0].sort()).toEqual([H("a"), H("b")]);
    // Cached: a second render never asks again.
    render(<ProductImage hash={H("a")} name="Cached" />);
    expect(screen.getAllByRole("img")).toHaveLength(4);
    expect(images).toHaveBeenCalledTimes(1);
  });

  it("falls back to the placeholder for unknown hashes, failed loads and broken pictures", async () => {
    images.mockResolvedValueOnce({});
    render(<ProductImage hash={H("c")} name="Unknown" />);
    await waitFor(() => expect(screen.getByTestId("product-image").dataset.state).toBe("placeholder"));

    images.mockRejectedValueOnce(new Error("offline"));
    const { unmount } = render(<ProductImage hash={H("d")} name="Offline" />);
    await waitFor(() => expect(screen.getAllByTestId("product-image")[1].dataset.state).toBe("placeholder"));
    unmount();

    images.mockResolvedValueOnce({ [H("e")]: PNG });
    render(<ProductImage hash={H("e")} name="Broken" />);
    const img = await screen.findByAltText("Broken");
    fireEvent.error(img);
    await waitFor(() => expect(screen.queryByAltText("Broken")).toBeNull());
  });

  it("makes two-letter monograms", () => {
    expect(productMonogram("Coke")).toBe("CO");
    expect(productMonogram("  pepsi max ")).toBe("PM");
    expect(productMonogram("")).toBe("");
  });
});
